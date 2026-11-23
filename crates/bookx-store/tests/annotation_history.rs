//! The annotation history in the catalogue: recorded in the same transaction as the book row,
//! restore through a normal write, retention, migration, backups.

mod common;

use std::path::PathBuf;

use bookx_core::annotation_journal::{
    self as journal, Action, ActionFilter, Filter, Outcome, Source, WriteContext,
};
use bookx_core::library::{Book, Bookmark, Highlight, Library};
use bookx_store::{open_library, SqliteStore, SCHEMA_VERSION};
use chrono::{Duration, TimeZone, Utc};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-hist-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hl(id: &str, start: i32, text: &str) -> Highlight {
    Highlight {
        id: id.into(),
        spine: 0,
        start,
        end: start + text.len() as i32,
        color: "yellow".into(),
        style: "background".into(),
        note: String::new(),
        text: text.into(),
        created: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
    }
}

fn mark(id: &str) -> Bookmark {
    Bookmark {
        id: id.into(),
        spine: 0,
        frac: 0.5,
        label: "Page 5".into(),
        created: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
    }
}

fn book(id: u64) -> Book {
    common::book(id, &format!("Book {id}"), "Ann Author")
}

/// A library with one book that has three highlights, a note and a bookmark.
fn setup(tag: &str) -> (PathBuf, bookx_store::OpenedLibrary, u64) {
    let root = tmp(tag);
    let mut o = open_library(&root).unwrap();
    let mut b = book(1);
    b.highlights = vec![
        hl("a", 0, "first"),
        hl("b", 10, "second"),
        hl("c", 20, "third"),
    ];
    b.highlights[1].note = "my note".into();
    b.bookmarks = vec![mark("m")];
    o.library.books.push(b);
    o.library.save().unwrap();
    (root, o, 1)
}

fn delete(lib: &mut Library, id: u64, ids: &[&str], source: Source) {
    lib.get_mut(id)
        .unwrap()
        .highlights
        .retain(|h| !ids.contains(&h.id.as_str()));
    lib.with_source(source, || lib.save()).unwrap();
}

#[test]
fn a_bulk_device_delete_is_one_group_with_who_and_the_whole_record() {
    let (root, mut o, id) = setup("bulk");
    assert_eq!(
        o.store.history_count(),
        0,
        "the first save adds a book, nothing is lost"
    );
    delete(
        &mut o.library,
        id,
        &["a", "b"],
        Source::Device("Kobo Libra".into()),
    );
    let es = o.store.history(&Filter::default(), 100).unwrap();
    assert_eq!(es.len(), 2);
    assert!(es.iter().all(|e| e.action == Action::Deleted
        && e.source == Source::Device("Kobo Libra".into())
        && e.title == "Book 1"
        && e.authors == "Ann Author"));
    assert_eq!(es[0].group, es[1].group);
    let b = es.iter().find(|e| e.record.id() == "b").unwrap();
    assert_eq!(b.kind, journal::Kind::Note);
    assert_eq!(b.record.note(), "my note");
    assert_eq!(o.store.history_devices(), vec!["Kobo Libra".to_string()]);
    // a delete by the reader is "you"
    delete(&mut o.library, id, &["c"], Source::You);
    let you = Filter {
        source: Some(Source::You),
        ..Filter::default()
    };
    assert_eq!(o.store.history(&you, 10).unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn per_book_sources_and_edits_and_bookmarks() {
    let (root, mut o, id) = setup("edit");
    o.library.get_mut(id).unwrap().highlights[0].note = "added later".into();
    o.library.get_mut(id).unwrap().bookmarks.clear();
    let mut ctx = WriteContext::by(Source::Merge);
    ctx.per_book.insert(id, Source::Import);
    o.library.save_with(&ctx).unwrap();
    let es = o.store.history(&Filter::default(), 10).unwrap();
    assert_eq!(es.len(), 2);
    assert!(es.iter().all(|e| e.source == Source::Import));
    assert!(es
        .iter()
        .any(|e| e.action == Action::Edited && e.record.note().is_empty()));
    assert!(es
        .iter()
        .any(|e| e.action == Action::Deleted && e.kind == journal::Kind::Bookmark));
    let edited = Filter {
        action: ActionFilter::Edited,
        ..Filter::default()
    };
    assert_eq!(o.store.history(&edited, 10).unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn restore_is_a_normal_write_that_is_journalled_and_can_be_undone() {
    let (root, mut o, id) = setup("restore");
    let before = o.library.get(id).unwrap().highlights.clone();
    delete(
        &mut o.library,
        id,
        &["a", "b", "c"],
        Source::Device("Kobo".into()),
    );
    let deleted = o.store.history(&Filter::default(), 10).unwrap();
    let mark = o.store.history_mark();
    for e in &deleted {
        assert_eq!(
            journal::restore(e, o.library.get_mut(id)),
            Outcome::Restored
        );
    }
    o.library
        .with_source(Source::Undo, || o.library.save())
        .unwrap();
    let mut got = o.library.get(id).unwrap().highlights.clone();
    got.sort_by_key(|h| h.start);
    assert_eq!(got, before, "byte for byte");
    // the restore itself is in the history, as additions by undo
    let added = o.store.history_since(mark).unwrap();
    assert_eq!(added.len(), 3);
    assert!(added
        .iter()
        .all(|e| e.action == Action::Added && e.source == Source::Undo));
    // ... so it can be undone: revert the additions
    let mark2 = o.store.history_mark();
    for e in &added {
        assert_eq!(journal::revert(e, o.library.get_mut(id)), Outcome::Removed);
    }
    o.library
        .with_source(Source::Undo, || o.library.save())
        .unwrap();
    assert!(o.library.get(id).unwrap().highlights.is_empty());
    assert_eq!(o.store.history_since(mark2).unwrap().len(), 3);
    // and what was stored is what a reopened library sees
    drop(o);
    let again = open_library(&root).unwrap();
    assert!(again.library.get(id).unwrap().highlights.is_empty());
    assert!(again.store.history_count() >= 9);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn history_outlives_the_book_and_never_blocks_deleting_it() {
    let (root, mut o, id) = setup("gone");
    delete(&mut o.library, id, &["a"], Source::You);
    o.library.remove(&[id], false).unwrap();
    let es = o.store.history(&Filter::default(), 10).unwrap();
    assert_eq!(es.len(), 1);
    assert_eq!(es[0].title, "Book 1");
    assert_eq!(
        journal::restore(&es[0], o.library.get_mut(id)),
        Outcome::BookMissing
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unchanged_annotations_and_new_books_write_nothing() {
    let (root, mut o, id) = setup("quiet");
    o.library.get_mut(id).unwrap().progress = 0.5;
    o.library.save().unwrap();
    let mut other = book(2);
    other.highlights = vec![hl("x", 0, "new book")];
    o.library.books.push(other);
    o.library.save().unwrap();
    assert_eq!(o.store.history_count(), 0);
    // a position that only moved is not a change either
    o.library.get_mut(id).unwrap().highlights[0].start = 3;
    o.library.save().unwrap();
    assert_eq!(o.store.history_count(), 0);
    let _ = std::fs::remove_dir_all(root);
}

fn insert_at(db: &std::path::Path, n: usize, days_ago: i64, text_len: usize) {
    // direct rows: the retention rule is about rows, whoever wrote them
    let at = (Utc::now() - Duration::days(days_ago)).timestamp_millis();
    let rec = serde_json::to_string(&journal::Record::Highlight(hl(
        "h",
        0,
        &"x".repeat(text_len),
    )))
    .unwrap();
    let conn = rusqlite::Connection::open(db).unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    for i in 0..n {
        tx.execute(
            "INSERT INTO annotation_history(at, book_id, title, authors, kind, action, source, group_id, annotation_id, record)
             VALUES(?1, 1, 'T', 'A', 'highlight', 'deleted', 'you', ?2, 'h', ?3)",
            rusqlite::params![at, i as i64, rec],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn retention_keeps_90_days_and_the_newest_500() {
    let root = tmp("ret");
    let db = root.join("c.db");
    let s = SqliteStore::open(&db).unwrap();
    insert_at(&db, 700, 400, 5);
    insert_at(&db, 10, 1, 5);
    assert_eq!(s.history_count(), 710);
    s.history_prune(Utc::now()).unwrap();
    assert_eq!(s.history_count(), journal::KEEP_MIN);
    // all recent entries stay even beyond 500
    insert_at(&db, 700, 3, 5);
    s.history_prune(Utc::now()).unwrap();
    assert_eq!(s.history_count(), 710);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn retention_has_hard_caps_on_rows_and_bytes() {
    let root = tmp("cap");
    let db = root.join("c.db");
    let s = SqliteStore::open(&db).unwrap();
    insert_at(&db, journal::MAX_ENTRIES + 25, 1, 5);
    s.history_prune(Utc::now()).unwrap();
    assert_eq!(s.history_count(), journal::MAX_ENTRIES);
    s.history_clear().unwrap();
    // ~200 KiB records: only what fits the byte cap stays
    insert_at(&db, 120, 1, 200 * 1024);
    s.history_prune(Utc::now()).unwrap();
    let n = s.history_count();
    assert!((70..120).contains(&n), "{n}");
    s.history_compact().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn old_databases_migrate_and_keep_working() {
    let root = tmp("mig");
    let db = root.join("c.db");
    {
        let (r, mut o, id) = setup("mig-src");
        delete(&mut o.library, id, &["a"], Source::You);
        let _ = std::fs::remove_dir_all(r);
    }
    {
        let s = SqliteStore::open(&db).unwrap();
        s.put_many(&[book(5)]).unwrap();
    }
    // a database from the previous schema has no history table
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch("DROP TABLE annotation_history; PRAGMA user_version = 9;")
            .unwrap();
    }
    let s = SqliteStore::open(&db).unwrap();
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    assert_eq!(s.history_count(), 0);
    assert_eq!(s.all_books().unwrap().len(), 1);
    // idempotent: the table exists already
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch("PRAGMA user_version = 9;").unwrap();
    }
    assert_eq!(
        SqliteStore::open(&db).unwrap().schema_version(),
        SCHEMA_VERSION
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_json_migrated_library_gets_the_table_and_records() {
    let root = tmp("json");
    let mut b = book(1);
    b.highlights = vec![hl("a", 0, "from json")];
    let json = serde_json::json!({"next_id": 2, "books": [b]});
    std::fs::write(root.join("bookx-library.json"), json.to_string()).unwrap();
    let mut o = open_library(&root).unwrap();
    assert_eq!(o.migrated, Some(1));
    assert_eq!(o.store.history_count(), 0);
    delete(&mut o.library, 1, &["a"], Source::You);
    assert_eq!(o.store.history_count(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn backups_carry_the_history_and_the_exports_do_not() {
    let (root, mut o, id) = setup("bak");
    delete(&mut o.library, id, &["a"], Source::Device("Kobo".into()));
    let f = root.join("snap.db");
    o.store.backup_to(&f).unwrap();
    let copy = SqliteStore::open(&f).unwrap();
    assert_eq!(copy.history_count(), 1);
    // restoring a backup brings its history back
    let (r2, o2, _) = setup("bak2");
    o2.store.restore_from(&f).unwrap();
    assert_eq!(o2.store.history_count(), 1);
    // the book rows themselves hold no history
    let data = serde_json::to_string(&o.library.get(id).unwrap()).unwrap();
    assert!(!data.contains("Kobo"));
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(r2);
}

#[test]
fn a_damaged_record_is_skipped_not_fatal() {
    let (root, mut o, id) = setup("dmg");
    delete(&mut o.library, id, &["a", "c"], Source::You);
    let conn = rusqlite::Connection::open(bookx_store::library_db_path(&root)).unwrap();
    conn.execute(
        "UPDATE annotation_history SET record = '{broken' WHERE annotation_id = 'a'",
        [],
    )
    .unwrap();
    let es = o.store.history(&Filter::default(), 10).unwrap();
    assert_eq!(es.len(), 1);
    assert_eq!(es[0].record.id(), "c");
    let _ = std::fs::remove_dir_all(root);
}
