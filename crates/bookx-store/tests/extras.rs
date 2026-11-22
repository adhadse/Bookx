//! JSON catalogue migration, collections, reading status, backup / restore / integrity, registry.

mod common;

use bookx_core::contracts::Store;
use bookx_store::{LibraryRegistry, ReadingStatus, SqliteStore};
use common::*;
use serde_json::json;

// -- JSON catalogue --------------------------------------------------------------------------

/// Hand-written catalogue in the format of `bookx-core`'s library module.
fn catalog_json(next_id: u64, ids: &[u64]) -> String {
    let books: Vec<_> = ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "file": format!("A/B ({id})/B.epub"),
                "meta": {
                    "title": format!("Book {id}"), "title_sort": format!("Book {id}"),
                    "authors": ["Ann"], "author_sort": "Ann", "publisher": "", "language": "en",
                    "description": "", "tags": ["t"], "series": "", "series_index": 0.0,
                    "identifiers": [{"scheme": "isbn", "value": "1"}],
                    "published": "", "rights": "", "rating": 4
                },
                "added": "2024-01-02T03:04:05Z",
                "modified": "2024-01-02T03:04:05Z",
                "size": 10
            })
        })
        .collect();
    json!({"next_id": next_id, "books": books}).to_string()
}

#[test]
fn json_catalog_import_preserves_ids_and_is_idempotent() {
    let t = Tmp::new();
    let file = t.join("bookx-library.json");
    std::fs::write(&file, catalog_json(50, &[3, 9, 12])).unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    let r = s.import_json_catalog(&file).unwrap();
    assert_eq!((r.imported, r.skipped), (3, 0));
    assert_eq!(s.ids(), vec![3, 9, 12]);
    assert_eq!(s.next_id(), 50);
    let b = s.get(9).unwrap();
    assert_eq!(b.meta.title, "Book 9");
    assert_eq!(b.meta.identifiers[0].value, "1");
    assert_eq!(b.meta.rating, 4);
    assert_eq!(b.progress, 0.0, "serde defaults apply");
    assert_eq!(s.search_ids("book 9"), vec![9]);

    let r = s.import_json_catalog(&file).unwrap();
    assert_eq!((r.imported, r.skipped), (0, 3));
    assert_eq!(s.len(), 3);
    assert_eq!(s.next_id(), 50);
}

#[test]
fn json_import_does_not_overwrite_existing_state() {
    let t = Tmp::new();
    let file = t.join("c.json");
    std::fs::write(&file, catalog_json(5, &[1, 2])).unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    let mut mine = book(1, "Kept", "Me");
    mine.progress = 0.5;
    s.put(mine).unwrap();
    let r = s.import_json_catalog(&file).unwrap();
    assert_eq!((r.imported, r.skipped), (1, 1));
    assert_eq!(s.get(1).unwrap().meta.title, "Kept");
    assert_eq!(s.next_id(), 5);
}

#[test]
fn json_import_with_duplicate_ids_inside_the_file() {
    let t = Tmp::new();
    let file = t.join("c.json");
    std::fs::write(&file, catalog_json(10, &[4, 4, 5])).unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    let r = s.import_json_catalog(&file).unwrap();
    assert_eq!((r.imported, r.skipped), (2, 1));
    assert_eq!(s.ids(), vec![4, 5]);
}

#[test]
fn json_import_never_lowers_next_id() {
    let t = Tmp::new();
    let file = t.join("c.json");
    std::fs::write(&file, catalog_json(2, &[1])).unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(book(20, "x", "y")).unwrap();
    s.import_json_catalog(&file).unwrap();
    assert_eq!(s.next_id(), 21);
}

#[test]
fn json_import_empty_catalogue_and_errors() {
    let t = Tmp::new();
    let s = SqliteStore::open_in_memory().unwrap();
    let file = t.join("empty.json");
    std::fs::write(&file, "{}").unwrap();
    assert_eq!(s.import_json_catalog(&file).unwrap().imported, 0);
    assert_eq!(s.next_id(), 1);

    let err = s.import_json_catalog(&t.join("missing.json")).unwrap_err();
    assert!(
        format!("{err:#}").contains("cannot read catalogue"),
        "{err:#}"
    );

    std::fs::write(&file, "{ not json").unwrap();
    let err = s.import_json_catalog(&file).unwrap_err();
    assert!(
        format!("{err:#}").contains("corrupt library catalogue"),
        "{err:#}"
    );
    assert!(s.is_empty());
}

#[test]
fn json_import_failure_rolls_back() {
    let t = Tmp::new();
    let file = t.join("c.json");
    let big = catalog_json(10, &[1, 2]).replace("\"id\":2", &format!("\"id\":{}", u64::MAX));
    std::fs::write(&file, big).unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    assert!(s.import_json_catalog(&file).is_err());
    assert!(s.is_empty());
}

// -- collections -----------------------------------------------------------------------------

fn three_books() -> SqliteStore {
    let s = SqliteStore::open_in_memory().unwrap();
    s.put_many(&[book(1, "a", "x"), book(2, "b", "x"), book(3, "c", "x")])
        .unwrap();
    s
}

#[test]
fn collections_create_list_rename_delete() {
    let s = three_books();
    assert!(s.list_collections().unwrap().is_empty());
    let fav = s.create_collection("Favourites").unwrap();
    let sci = s.create_collection("  Sci-Fi ").unwrap();
    assert_ne!(fav, sci);
    let list = s.list_collections().unwrap();
    assert_eq!(
        list.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["Favourites", "Sci-Fi"]
    );
    s.rename_collection(sci, "Science Fiction").unwrap();
    assert_eq!(s.list_collections().unwrap()[1].name, "Science Fiction");
    assert!(s.delete_collection(fav).unwrap());
    assert!(!s.delete_collection(fav).unwrap());
    assert_eq!(s.list_collections().unwrap().len(), 1);
}

#[test]
fn collection_name_errors() {
    let s = three_books();
    let a = s.create_collection("Mine").unwrap();
    let b = s.create_collection("Other").unwrap();
    assert!(
        s.create_collection("  ")
            .unwrap_err()
            .to_string()
            .contains("empty")
    );
    assert!(
        s.create_collection("mine")
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert!(
        s.rename_collection(b, "MINE")
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert!(
        s.rename_collection(b, "")
            .unwrap_err()
            .to_string()
            .contains("empty")
    );
    assert!(
        s.rename_collection(999, "Zed")
            .unwrap_err()
            .to_string()
            .contains("no such collection")
    );
    // renaming to a different case of itself is fine
    s.rename_collection(a, "MINE").unwrap();
    assert_eq!(s.list_collections().unwrap()[0].name, "MINE");
}

#[test]
fn books_in_many_collections() {
    let s = three_books();
    let c1 = s.create_collection("One").unwrap();
    let c2 = s.create_collection("Two").unwrap();
    s.add_to_collection(c1, 1).unwrap();
    s.add_to_collection(c1, 2).unwrap();
    s.add_to_collection(c2, 2).unwrap();
    s.add_to_collection(c1, 2).unwrap(); // idempotent
    assert_eq!(s.collection_books(c1), vec![1, 2]);
    assert_eq!(s.collection_books(c2), vec![2]);
    assert_eq!(s.collections_of(2), vec![c1, c2]);
    assert!(s.collections_of(3).is_empty());
    let counts: Vec<_> = s
        .list_collections()
        .unwrap()
        .iter()
        .map(|c| c.books)
        .collect();
    assert_eq!(counts, vec![2, 1]);

    assert!(s.remove_from_collection(c1, 2).unwrap());
    assert!(!s.remove_from_collection(c1, 2).unwrap());
    assert!(!s.remove_from_collection(c1, u64::MAX).unwrap());
    assert_eq!(s.collection_books(c1), vec![1]);
    assert_eq!(s.collection_books(c2), vec![2]);
    assert!(s.collection_books(12345).is_empty());
}

#[test]
fn collection_membership_errors() {
    let s = three_books();
    let c = s.create_collection("C").unwrap();
    assert!(
        s.add_to_collection(c, 99)
            .unwrap_err()
            .to_string()
            .contains("no such book 99")
    );
    assert!(
        s.add_to_collection(77, 1)
            .unwrap_err()
            .to_string()
            .contains("no such collection 77")
    );
}

#[test]
fn removing_a_book_or_collection_cleans_links() {
    let s = three_books();
    let c1 = s.create_collection("One").unwrap();
    let c2 = s.create_collection("Two").unwrap();
    s.add_to_collection(c1, 1).unwrap();
    s.add_to_collection(c2, 1).unwrap();
    s.add_to_collection(c2, 2).unwrap();
    s.set_status(1, ReadingStatus::Finished).unwrap();
    s.remove(1).unwrap();
    assert!(s.collection_books(c1).is_empty());
    assert_eq!(s.collection_books(c2), vec![2]);
    assert!(s.get_status(1).is_none());
    // updating a book (upsert) must not drop its links or status
    s.set_status(2, ReadingStatus::OnHold).unwrap();
    s.put(book(2, "renamed", "x")).unwrap();
    assert_eq!(s.collection_books(c2), vec![2]);
    assert_eq!(s.get_status(2), Some(ReadingStatus::OnHold));
    // deleting a collection keeps the books
    assert!(s.delete_collection(c2).unwrap());
    assert!(s.contains(2));
    assert!(s.collections_of(2).is_empty());
}

// -- status ----------------------------------------------------------------------------------

#[test]
fn status_defaults_to_new_and_roundtrips() {
    let s = three_books();
    assert_eq!(s.get_status(1), Some(ReadingStatus::New));
    assert_eq!(s.get_status(99), None);
    for st in ReadingStatus::ALL {
        s.set_status(1, st).unwrap();
        assert_eq!(s.get_status(1), Some(st));
    }
    let err = s.set_status(99, ReadingStatus::Reading).unwrap_err();
    assert!(err.to_string().contains("no such book 99"), "{err}");
}

#[test]
fn status_names_parse_both_ways() {
    for st in ReadingStatus::ALL {
        assert_eq!(ReadingStatus::parse(st.as_str()), Some(st));
    }
    assert_eq!(ReadingStatus::New.as_str(), "new");
    assert_eq!(ReadingStatus::OnHold.as_str(), "on_hold");
    assert_eq!(ReadingStatus::parse("bogus"), None);
    assert_eq!(ReadingStatus::parse("On_Hold"), None);
}

#[test]
fn books_with_status_lists_ids() {
    let s = three_books();
    s.set_status(2, ReadingStatus::Reading).unwrap();
    s.set_status(3, ReadingStatus::Reading).unwrap();
    s.set_status(3, ReadingStatus::Finished).unwrap();
    assert_eq!(s.books_with_status(ReadingStatus::Reading), vec![2]);
    assert_eq!(s.books_with_status(ReadingStatus::Finished), vec![3]);
    assert_eq!(s.books_with_status(ReadingStatus::New), vec![1]);
    assert!(s.books_with_status(ReadingStatus::OnHold).is_empty());
}

#[test]
fn status_and_collections_persist_on_disk() {
    let t = Tmp::new();
    let path = t.join("l.db");
    let c;
    {
        let s = SqliteStore::open(&path).unwrap();
        s.put(book(1, "a", "x")).unwrap();
        c = s.create_collection("Keep").unwrap();
        s.add_to_collection(c, 1).unwrap();
        s.set_status(1, ReadingStatus::OnHold).unwrap();
    }
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.collection_books(c), vec![1]);
    assert_eq!(s.get_status(1), Some(ReadingStatus::OnHold));
}

// -- backup / restore / integrity --------------------------------------------------------------

fn populated(path: &std::path::Path, n: u64) -> SqliteStore {
    let s = SqliteStore::open(path).unwrap();
    let books: Vec<_> = (1..=n)
        .map(|i| {
            let mut b = book(i, &format!("Book number {i}"), "Writer");
            b.meta.description = "lorem ipsum ".repeat(60);
            b
        })
        .collect();
    s.put_many(&books).unwrap();
    s
}

#[test]
fn backup_and_restore_roundtrip() {
    let t = Tmp::new();
    let s = populated(&t.join("live.db"), 20);
    s.put(full_book(100)).unwrap();
    let c = s.create_collection("Shelf").unwrap();
    s.add_to_collection(c, 100).unwrap();
    s.set_status(100, ReadingStatus::Reading).unwrap();
    s.index_book_text(100, "searchable zeppelin text").unwrap();

    let bak = t.join("nested/backup.db");
    s.backup_to(&bak).unwrap();
    assert!(bak.is_file());
    assert!(SqliteStore::check_file(&bak).is_empty());
    // no temp files left behind
    let leftovers: Vec<_> = std::fs::read_dir(t.join("nested"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // damage the live store, then restore
    for id in 1..=20 {
        s.remove(id).unwrap();
    }
    s.delete_collection(c).unwrap();
    assert_eq!(s.len(), 1);
    s.restore_from(&bak).unwrap();
    assert_eq!(s.len(), 21);
    assert!(same(&s.get(100).unwrap(), &full_book(100)));
    assert_eq!(s.collection_books(c), vec![100]);
    assert_eq!(s.get_status(100), Some(ReadingStatus::Reading));
    assert_eq!(s.search_text("zeppelin", 3).unwrap()[0].id, 100);
    assert_eq!(s.search_ids("title:\"=Book number 7\""), vec![7]);
    assert!(s.check_integrity().is_empty());
    // still writable afterwards
    s.put(book(500, "after", "x")).unwrap();
    assert!(s.contains(500));
}

#[test]
fn backup_overwrites_an_existing_file_and_works_in_memory() {
    let t = Tmp::new();
    let bak = t.join("b.db");
    std::fs::write(&bak, "old junk").unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(book(1, "mem", "x")).unwrap();
    s.backup_to(&bak).unwrap();
    let copy = SqliteStore::open(&bak).unwrap();
    assert_eq!(copy.ids(), vec![1]);
}

#[test]
fn restore_into_memory_store() {
    let t = Tmp::new();
    let bak = t.join("b.db");
    populated(&bak, 3);
    let s = SqliteStore::open_in_memory().unwrap();
    s.restore_from(&bak).unwrap();
    assert_eq!(s.len(), 3);
}

#[test]
fn restore_rejects_bad_backups_and_keeps_the_store() {
    let t = Tmp::new();
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(book(1, "keep", "x")).unwrap();

    let err = s.restore_from(&t.join("missing.db")).unwrap_err();
    assert!(format!("{err:#}").contains("cannot open backup"), "{err:#}");

    let junk = t.join("junk.db");
    std::fs::write(&junk, vec![7u8; 9000]).unwrap();
    let err = s.restore_from(&junk).unwrap_err();
    assert!(
        format!("{err:#}").contains("not a Bookx database"),
        "{err:#}"
    );

    let foreign = t.join("foreign.db");
    rusqlite::Connection::open(&foreign)
        .unwrap()
        .execute_batch("CREATE TABLE other(x);")
        .unwrap();
    let err = s.restore_from(&foreign).unwrap_err();
    assert!(err.to_string().contains("not a Bookx database"), "{err}");

    let future = t.join("future.db");
    {
        let c = rusqlite::Connection::open(&future).unwrap();
        c.execute_batch("CREATE TABLE books(id); PRAGMA user_version = 99;")
            .unwrap();
    }
    let err = s.restore_from(&future).unwrap_err();
    assert!(err.to_string().contains("newer than supported"), "{err}");

    assert_eq!(s.ids(), vec![1]);
    assert_eq!(s.get(1).unwrap().meta.title, "keep");
}

#[test]
fn restore_migrates_an_old_backup() {
    let t = Tmp::new();
    let old = t.join("old.db");
    {
        let s = SqliteStore::open(&old).unwrap();
        s.put(book(1, "oldie", "x")).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&old).unwrap();
        c.execute_batch(
            "DROP TABLE text_stamps; DROP TABLE collection_books; DROP TABLE collections;
             DROP TABLE book_status; PRAGMA user_version = 2;",
        )
        .unwrap();
    }
    let s = SqliteStore::open_in_memory().unwrap();
    s.restore_from(&old).unwrap();
    assert_eq!(s.schema_version(), bookx_store::SCHEMA_VERSION);
    assert!(s.create_collection("works").unwrap() > 0);
    assert_eq!(s.search_ids("oldie"), vec![1]);
}

#[test]
fn integrity_is_clean_for_healthy_databases() {
    let t = Tmp::new();
    let s = populated(&t.join("ok.db"), 30);
    assert!(s.check_integrity().is_empty());
    drop(s);
    assert!(SqliteStore::check_file(&t.join("ok.db")).is_empty());
}

#[test]
fn integrity_detects_a_corrupted_file() {
    let t = Tmp::new();
    let path = t.join("bad.db");
    drop(populated(&path, 400));
    let len = std::fs::metadata(&path).unwrap().len();
    assert!(len > 40_000, "fixture too small: {len}");
    let mut bytes = std::fs::read(&path).unwrap();
    // keep the 100 byte header intact, trash the pages after it
    let end = bytes.len().min(100 + 40_000);
    for b in &mut bytes[4096..end] {
        *b = 0xAB;
    }
    std::fs::write(&path, bytes).unwrap();
    let problems = SqliteStore::check_file(&path);
    assert!(!problems.is_empty(), "corruption not detected");
}

#[test]
fn integrity_check_on_non_database_reports_a_message() {
    let t = Tmp::new();
    let path = t.join("text.db");
    std::fs::write(
        &path,
        "this is not sqlite at all, just a text file".repeat(50),
    )
    .unwrap();
    let problems = SqliteStore::check_file(&path);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("not a database"), "{problems:?}");
    assert!(!SqliteStore::check_file(&t.join("absent.db")).is_empty());
}

#[test]
fn integrity_detects_index_drift() {
    let t = Tmp::new();
    let path = t.join("drift.db");
    drop(populated(&path, 5));
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM fts_meta WHERE rowid = 3", [])
        .unwrap();
    let problems = SqliteStore::check_file(&path);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("out of sync"), "{problems:?}");
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.check_integrity().len(), 1);
}

// -- registry --------------------------------------------------------------------------------

#[test]
fn registry_add_list_current() {
    let t = Tmp::new();
    let file = t.join("cfg/libraries.json");
    let mut r = LibraryRegistry::open(&file).unwrap();
    assert!(r.list().is_empty());
    assert!(r.current().is_none());
    r.add("Home", &t.join("home")).unwrap();
    r.add("Work", &t.join("work")).unwrap();
    assert_eq!(r.list().len(), 2);
    assert_eq!(
        r.current().unwrap().name,
        "Home",
        "first library becomes current"
    );
    r.set_current("Work").unwrap();
    assert_eq!(r.current().unwrap().path, t.join("work"));
    assert_eq!(r.get("Home").unwrap().path, t.join("home"));
    assert!(r.get("Nope").is_none());

    // persisted
    let r2 = LibraryRegistry::open(&file).unwrap();
    assert_eq!(r2.list(), r.list());
    assert_eq!(r2.current().unwrap().name, "Work");
}

#[test]
fn registry_validation_errors() {
    let t = Tmp::new();
    let mut r = LibraryRegistry::open(&t.join("l.json")).unwrap();
    r.add("A", t.path()).unwrap();
    assert!(
        r.add("A", t.path())
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert!(
        r.add("  ", t.path())
            .unwrap_err()
            .to_string()
            .contains("empty")
    );
    assert!(
        r.rename("Zed", "Y")
            .unwrap_err()
            .to_string()
            .contains("no library named")
    );
    assert!(r.rename("A", "").unwrap_err().to_string().contains("empty"));
    r.add("B", t.path()).unwrap();
    assert!(
        r.rename("A", "B")
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert!(
        r.set_current("Zed")
            .unwrap_err()
            .to_string()
            .contains("no library named")
    );
    assert_eq!(r.list().len(), 2);
}

#[test]
fn registry_rename_and_remove_track_the_current_library() {
    let t = Tmp::new();
    let file = t.join("l.json");
    let mut r = LibraryRegistry::open(&file).unwrap();
    r.add("A", &t.join("a")).unwrap();
    r.add("B", &t.join("b")).unwrap();
    r.rename("A", "Alpha").unwrap();
    assert_eq!(r.current().unwrap().name, "Alpha");
    r.rename("Alpha", "Alpha").unwrap();
    assert!(!r.remove("missing").unwrap());
    assert!(r.remove("Alpha").unwrap());
    assert_eq!(
        r.current().unwrap().name,
        "B",
        "falls back to a remaining library"
    );
    assert!(r.remove("B").unwrap());
    assert!(r.current().is_none());
    assert!(r.list().is_empty());
    assert!(LibraryRegistry::open(&file).unwrap().current().is_none());
    // removing never touches the files
    assert!(t.path().exists());
}

#[test]
fn registry_corrupt_file_is_an_error() {
    let t = Tmp::new();
    let file = t.join("l.json");
    std::fs::write(&file, "{{{").unwrap();
    let err = LibraryRegistry::open(&file).unwrap_err();
    assert!(
        format!("{err:#}").contains("corrupt library registry"),
        "{err:#}"
    );
    // a directory in place of the file is an IO error, not a panic
    let dir = t.join("dir");
    std::fs::create_dir(&dir).unwrap();
    assert!(LibraryRegistry::open(&dir).is_err());
}

#[test]
fn registry_accepts_hand_written_minimal_json() {
    let t = Tmp::new();
    let file = t.join("l.json");
    std::fs::write(&file, r#"{"libraries":[{"name":"X","path":"/tmp/x"}]}"#).unwrap();
    let r = LibraryRegistry::open(&file).unwrap();
    assert_eq!(r.list()[0].name, "X");
    assert!(r.current().is_none());
}

#[test]
fn registry_with_dangling_current_has_no_current() {
    let t = Tmp::new();
    let file = t.join("l.json");
    std::fs::write(&file, r#"{"current":"Gone","libraries":[]}"#).unwrap();
    assert!(LibraryRegistry::open(&file).unwrap().current().is_none());
}

#[test]
fn a_version_3_database_gains_the_text_stamp_table() {
    let t = Tmp::new();
    let old = t.join("v3.db");
    {
        let s = SqliteStore::open(&old).unwrap();
        s.put(book(1, "threeish", "x")).unwrap();
    }
    {
        let c = rusqlite::Connection::open(&old).unwrap();
        c.execute_batch("DROP TABLE text_stamps; PRAGMA user_version = 3;")
            .unwrap();
    }
    let s = SqliteStore::open(&old).unwrap();
    assert_eq!(s.schema_version(), bookx_store::SCHEMA_VERSION);
    assert!(s.text_stamps().is_empty());
    s.index_book_text_stamped(1, 7, "hello stamps").unwrap();
    assert_eq!(s.text_stamps().get(&1), Some(&7));
    assert_eq!(s.search_ids("threeish"), vec![1]);
}
