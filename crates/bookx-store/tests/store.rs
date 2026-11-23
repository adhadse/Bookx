mod common;

use std::sync::Arc;

use bookx_core::contracts::Store;
use bookx_store::{Order, SCHEMA_VERSION, SqliteStore};
use common::*;
use rusqlite::Connection;

#[test]
fn empty_store() {
    let s = SqliteStore::open_in_memory().unwrap();
    assert!(s.is_empty());
    assert_eq!(s.len(), 0);
    assert!(s.ids().is_empty());
    assert!(s.get(1).is_none());
    assert!(!s.remove(1).unwrap());
    assert!(s.search_ids("").is_empty());
    assert!(s.search_ids("anything").is_empty());
    assert!(s.ids_sorted(Order::Title, true).is_empty());
    assert_eq!(s.next_id(), 1);
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    assert!(s.check_integrity().is_empty());
}

#[test]
fn roundtrips_every_field() {
    let s = SqliteStore::open_in_memory().unwrap();
    let b = full_book(42);
    s.put(b.clone()).unwrap();
    let back = s.get(42).unwrap();
    assert!(same(&b, &back), "{b:?}\n{back:?}");
    assert_eq!(back.meta.identifiers[0].xml_id.as_deref(), Some("bookid"));
    assert_eq!(back.position.unwrap().spine, 7);
    assert_eq!(back.highlights[0].start, -4);
    assert_eq!(back.last_read, b.last_read);
    assert_eq!(back.added, b.added);
}

#[test]
fn survives_reopen_on_disk() {
    let t = Tmp::new();
    let path = t.join("sub/dir/lib.db");
    {
        let s = SqliteStore::open(&path).unwrap();
        s.put(full_book(5)).unwrap();
    }
    let s = SqliteStore::open(&path).unwrap();
    assert!(same(&s.get(5).unwrap(), &full_book(5)));
    assert_eq!(s.next_id(), 6);
}

#[test]
fn put_replaces_duplicate_id() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(book(1, "First", "A")).unwrap();
    s.put(book(1, "Second", "B")).unwrap();
    assert_eq!(s.ids(), vec![1]);
    assert_eq!(s.get(1).unwrap().meta.title, "Second");
    // the old title is gone from the index
    assert!(s.search_ids("First").is_empty());
    assert_eq!(s.search_ids("Second"), vec![1]);
}

#[test]
fn remove_reports_presence() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(book(1, "A", "x")).unwrap();
    assert!(s.remove(1).unwrap());
    assert!(!s.remove(1).unwrap());
    assert!(s.get(1).is_none());
    assert!(s.is_empty());
}

#[test]
fn ids_are_sorted_ascending() {
    let s = SqliteStore::open_in_memory().unwrap();
    for id in [9, 3, 7, 1] {
        s.put(book(id, "t", "a")).unwrap();
    }
    assert_eq!(s.ids(), vec![1, 3, 7, 9]);
    assert!(s.contains(7) && !s.contains(8));
}

#[test]
fn huge_ids_are_rejected_not_wrapped() {
    let s = SqliteStore::open_in_memory().unwrap();
    let err = s.put(book(u64::MAX, "t", "a")).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    assert!(s.get(u64::MAX).is_none());
    assert!(!s.remove(u64::MAX).unwrap());
    assert!(!s.contains(u64::MAX));
    assert!(s.is_empty());
}

#[test]
fn unicode_titles_and_authors() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(book(1, "Крамола ☃ 日本語の本", "Достоевский"))
        .unwrap();
    s.put(book(2, "Ünïcödé", "Zoë")).unwrap();
    assert_eq!(s.get(1).unwrap().meta.title, "Крамола ☃ 日本語の本");
    assert_eq!(s.search_ids("Достоевский"), vec![1]);
    assert_eq!(s.search_ids("日本語"), vec![1]);
    assert_eq!(s.search_ids("ünï"), vec![2]);
}

#[test]
fn next_id_and_allocation() {
    let s = SqliteStore::open_in_memory().unwrap();
    assert_eq!(s.allocate_id().unwrap(), 1);
    assert_eq!(s.allocate_id().unwrap(), 2);
    s.put(book(10, "t", "a")).unwrap();
    assert_eq!(s.next_id(), 11);
    s.put(book(3, "t", "a")).unwrap();
    assert_eq!(s.next_id(), 11, "never goes down");
    assert_eq!(s.allocate_id().unwrap(), 11);
    s.remove(10).unwrap();
    assert_eq!(s.next_id(), 12, "ids are not reused");
}

#[test]
fn put_many_is_atomic() {
    let s = SqliteStore::open_in_memory().unwrap();
    let books = vec![
        book(1, "a", "x"),
        book(u64::MAX, "bad", "x"),
        book(3, "c", "x"),
    ];
    assert!(s.put_many(&books).is_err());
    assert!(s.is_empty(), "failed batch left rows behind");
    s.put_many(&[book(1, "a", "x"), book(3, "c", "x")]).unwrap();
    assert_eq!(s.ids(), vec![1, 3]);
    assert_eq!(s.all_books().unwrap().len(), 2);
}

#[test]
fn sorted_ids_follow_each_order() {
    let s = SqliteStore::open_in_memory().unwrap();
    let mut a = book(1, "banana", "Zed");
    a.meta.series = "S2".into();
    a.meta.series_index = 2.0;
    a.added = ts(300);
    a.last_read = Some(ts(50));
    let mut b = book(2, "Apple", "yan");
    b.meta.series = "s1".into();
    b.meta.series_index = 1.0;
    b.added = ts(100);
    let mut c = book(3, "cherry", "Xu");
    c.meta.series = "S2".into();
    c.meta.series_index = 1.0;
    c.added = ts(200);
    c.last_read = Some(ts(10));
    s.put_many(&[a, b, c]).unwrap();
    assert_eq!(s.ids_sorted(Order::Title, true), vec![2, 1, 3]);
    assert_eq!(s.ids_sorted(Order::Title, false), vec![3, 1, 2]);
    assert_eq!(s.ids_sorted(Order::Author, true), vec![3, 2, 1]);
    assert_eq!(s.ids_sorted(Order::Series, true), vec![2, 3, 1]);
    assert_eq!(s.ids_sorted(Order::Added, true), vec![2, 3, 1]);
    assert_eq!(s.ids_sorted(Order::Added, false), vec![1, 3, 2]);
    // never-read books sort first ascending, like Option::None
    assert_eq!(s.ids_sorted(Order::LastRead, true), vec![2, 3, 1]);
    assert_eq!(s.ids_sorted(Order::LastRead, false), vec![1, 3, 2]);
}

#[test]
fn eight_threads_put_and_get() {
    let s = Arc::new(SqliteStore::open_in_memory().unwrap());
    let handles: Vec<_> = (0..8u64)
        .map(|t| {
            let s = s.clone();
            std::thread::spawn(move || {
                for i in 0..50u64 {
                    let id = t * 1000 + i + 1;
                    s.put(book(id, &format!("Title {id}"), "Thread")).unwrap();
                    let got = s.get(id).unwrap();
                    assert_eq!(got.meta.title, format!("Title {id}"));
                    s.search_ids("Thread");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(s.len(), 400);
    assert_eq!(s.ids().len(), 400);
    assert!(s.check_integrity().is_empty());
}

#[test]
fn eight_threads_on_a_file_with_separate_handles() {
    let t = Tmp::new();
    let path = t.join("lib.db");
    SqliteStore::open(&path).unwrap();
    let handles: Vec<_> = (0..8u64)
        .map(|n| {
            let path = path.clone();
            std::thread::spawn(move || {
                let s = SqliteStore::open(&path).unwrap();
                for i in 0..20u64 {
                    s.put(book(n * 100 + i + 1, "x", "y")).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(SqliteStore::open(&path).unwrap().len(), 160);
}

// -- migrations ----------------------------------------------------------

fn user_version(path: &std::path::Path) -> i32 {
    Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn new_file_is_migrated_from_v0_to_latest() {
    let t = Tmp::new();
    let path = t.join("lib.db");
    // a completely empty sqlite file has user_version 0
    Connection::open(&path).unwrap();
    assert_eq!(user_version(&path), 0);
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    drop(s);
    assert_eq!(user_version(&path), SCHEMA_VERSION);
    let c = Connection::open(&path).unwrap();
    let mode: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    for idx in [
        "books_title_sort",
        "books_author_sort",
        "books_series",
        "books_added",
        "books_last_read",
    ] {
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [idx],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "missing index {idx}");
    }
}

#[test]
fn reopening_is_idempotent() {
    let t = Tmp::new();
    let path = t.join("lib.db");
    SqliteStore::open(&path)
        .unwrap()
        .put(book(1, "a", "b"))
        .unwrap();
    for _ in 0..3 {
        let s = SqliteStore::open(&path).unwrap();
        assert_eq!(s.ids(), vec![1]);
    }
}

/// Builds a database exactly as version 1 of the app wrote it (hand-written SQL, independent of the
/// migration code) and checks that opening upgrades it and keeps the data usable.
#[test]
fn upgrades_a_v1_database_and_backfills_the_search_index() {
    let t = Tmp::new();
    let path = t.join("old.db");
    {
        let c = Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             INSERT INTO meta VALUES('next_id', 8);
             CREATE TABLE books(id INTEGER PRIMARY KEY, title TEXT NOT NULL, title_sort TEXT NOT NULL,
                 author_sort TEXT NOT NULL, series TEXT NOT NULL, series_index REAL NOT NULL,
                 added INTEGER NOT NULL, modified INTEGER NOT NULL, last_read INTEGER,
                 progress REAL NOT NULL, data TEXT NOT NULL);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        let b = book(7, "Legacy Book", "Old Author");
        c.execute(
            "INSERT INTO books VALUES(7,'Legacy Book','Legacy Book','Old Author','',0,1,1,NULL,0,?1)",
            [serde_json::to_string(&b).unwrap()],
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    assert_eq!(s.get(7).unwrap().meta.title, "Legacy Book");
    assert_eq!(s.search_ids("legacy"), vec![7], "FTS was not backfilled");
    assert_eq!(s.next_id(), 8);
    let c = s.create_collection("Old").unwrap();
    s.add_to_collection(c, 7).unwrap();
    assert_eq!(s.collection_books(c), vec![7]);
    assert!(s.check_integrity().is_empty());
}

#[test]
fn upgrades_a_v2_database_with_collections() {
    let t = Tmp::new();
    let path = t.join("v2.db");
    {
        let s = SqliteStore::open(&path).unwrap();
        s.put(book(1, "Two", "x")).unwrap();
    }
    {
        let c = Connection::open(&path).unwrap();
        c.execute_batch(
            "DROP TABLE text_stamps; DROP TABLE collection_books; DROP TABLE collections;
             DROP TABLE book_status; PRAGMA user_version = 2;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    assert_eq!(s.list_collections().unwrap().len(), 0);
    assert_eq!(s.search_ids("Two"), vec![1]);
}

#[test]
fn refuses_a_database_from_the_future() {
    let t = Tmp::new();
    let path = t.join("future.db");
    {
        let c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA user_version = 99;").unwrap();
    }
    let err = SqliteStore::open(&path).err().unwrap();
    assert!(
        format!("{err:#}").contains("newer than supported"),
        "{err:#}"
    );
}

#[test]
fn garbage_file_is_an_error_not_a_panic() {
    let t = Tmp::new();
    let path = t.join("garbage.db");
    std::fs::write(&path, vec![0x42u8; 5000]).unwrap();
    let err = SqliteStore::open(&path).err().unwrap();
    assert!(format!("{err:#}").contains("garbage.db"), "{err:#}");
}

#[test]
fn unreadable_book_json_is_skipped_not_fatal() {
    let t = Tmp::new();
    let path = t.join("lib.db");
    {
        let s = SqliteStore::open(&path).unwrap();
        s.put(book(1, "ok", "a")).unwrap();
        s.put(book(2, "bad", "a")).unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE books SET data = '{nope' WHERE id = 2", [])
        .unwrap();
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.ids(), vec![1, 2]);
    assert!(s.get(2).is_none());
    assert_eq!(s.all_books().unwrap().len(), 1);
}
