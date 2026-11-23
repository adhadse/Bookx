//! Schema v6: several format files per book and author aliases; backups and old databases.

mod common;

use bookx_core::contracts::Store;
use bookx_core::formats::FormatFile;
use bookx_store::{SCHEMA_VERSION, SqliteStore};
use common::*;
use rusqlite::Connection;

fn two_formats(id: u64) -> bookx_core::library::Book {
    let mut b = book(id, "Dune", "Frank Herbert");
    b.formats.push(FormatFile {
        file: "Frank Herbert/Dune/Dune.azw3".into(),
        format: "azw3".into(),
        size: 77,
    });
    b
}

#[test]
fn format_rows_follow_the_book() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(two_formats(1)).unwrap();
    s.put(book(2, "Other", "X")).unwrap();
    let f = s.formats_of(1);
    assert_eq!(f.len(), 2);
    assert_eq!(f[0].1, "epub");
    assert!(f[0].3, "the main file comes first");
    assert_eq!(f[1].1, "azw3");
    assert_eq!(s.books_with_format("AZW3"), vec![1]);
    assert_eq!(s.books_with_format("epub"), vec![1, 2]);
    // dropping the extra format drops its row
    s.put(book(1, "Dune", "Frank Herbert")).unwrap();
    assert!(s.books_with_format("azw3").is_empty());
    // removing the book removes its rows
    s.remove(1).unwrap();
    assert!(s.formats_of(1).is_empty());
}

#[test]
fn a_v5_database_gets_format_rows_for_its_books() {
    let t = Tmp::new();
    let path = t.join("v5.db");
    {
        let s = SqliteStore::open(&path).unwrap();
        s.put(two_formats(3)).unwrap();
    }
    {
        let c = Connection::open(&path).unwrap();
        c.execute_batch(
            "DROP TABLE book_formats; DROP TABLE author_aliases; PRAGMA user_version = 5;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.schema_version(), SCHEMA_VERSION);
    assert_eq!(
        s.formats_of(3).len(),
        2,
        "rows backfilled from the catalogue"
    );
    assert!(s.author_aliases().is_empty());
    assert!(s.check_integrity().is_empty());
    assert_eq!(
        s.get(3).unwrap().formats.len(),
        1,
        "the book data is untouched"
    );
}

#[test]
fn backups_carry_formats_and_aliases_and_old_backups_still_restore() {
    let t = Tmp::new();
    let s = SqliteStore::open(&t.join("a.db")).unwrap();
    s.put(two_formats(1)).unwrap();
    s.set_author_alias("F. Herbert", "Frank Herbert").unwrap();
    s.backup_to(&t.join("b.db")).unwrap();
    // an older (v5) backup
    {
        let c = Connection::open(t.join("b.db")).unwrap();
        c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    }
    let old = t.join("old.db");
    s.backup_to(&old).unwrap();
    Connection::open(&old)
        .unwrap()
        .execute_batch(
            "DROP TABLE book_formats; DROP TABLE author_aliases; PRAGMA user_version = 5;",
        )
        .unwrap();

    let fresh = SqliteStore::open(&t.join("c.db")).unwrap();
    fresh.restore_from(&t.join("b.db")).unwrap();
    assert_eq!(fresh.formats_of(1).len(), 2);
    assert_eq!(fresh.author_aliases().len(), 1);

    let from_old = SqliteStore::open(&t.join("d.db")).unwrap();
    from_old.restore_from(&old).unwrap();
    assert_eq!(from_old.schema_version(), SCHEMA_VERSION);
    assert_eq!(from_old.formats_of(1).len(), 2);
    assert!(from_old.author_aliases().is_empty());
}

#[test]
fn aliases_are_case_insensitive_and_never_chain() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.set_author_alias("Mark Twain", "Samuel Clemens").unwrap();
    s.set_author_alias("S. L. Clemens", "Samuel Clemens")
        .unwrap();
    // Samuel Clemens becomes an alias of Twain: everything that pointed at him follows
    s.set_author_alias("Samuel Clemens", "Mark Twain")
        .unwrap_err(); // Mark Twain is his alias
    let a = s.author_aliases();
    assert_eq!(a.len(), 2);
    assert!(a.iter().all(|(_, c)| c == "Samuel Clemens"));
    // making the canonical name an alias of a new name moves the others along
    s.set_author_alias("Samuel Clemens", "Samuel Langhorne Clemens")
        .unwrap();
    let a = s.author_aliases();
    assert_eq!(a.len(), 3);
    assert!(a.iter().all(|(_, c)| c == "Samuel Langhorne Clemens"));
    s.set_author_alias("MARK TWAIN", "Samuel Langhorne Clemens")
        .unwrap();
    assert_eq!(
        s.author_aliases().len(),
        3,
        "same alias in other case replaces"
    );
    assert!(s.set_author_alias("x", "X").is_err());
    assert!(s.set_author_alias("", "X").is_err());
    assert!(s.remove_author_alias("mark twain").unwrap());
    assert_eq!(s.author_aliases().len(), 2);
}

#[test]
fn merging_formats_carries_collections_status_and_progress_and_refuses_cleanly() {
    use bookx_core::library::{AddOutcome, Library};
    use bookx_store::ReadingStatus;
    let t = Tmp::new();
    let store = SqliteStore::open_in_memory().unwrap();
    let mut lib = Library::in_memory(&t.join("lib")).unwrap();
    let mut add = |title: &str| {
        let p = t.join(&format!("{title}.md"));
        std::fs::write(
            &p,
            format!("---\ntitle: \"{title}\"\nauthor: \"A\"\n---\n# One\n\nHi.\n"),
        )
        .unwrap();
        match lib.add_path(&p, true).unwrap() {
            AddOutcome::Added(id) => id,
            AddOutcome::Duplicate(_) => panic!(),
        }
    };
    let (a, b, c) = (add("One"), add("Two"), add("Three"));
    let old = lib.get(b).unwrap().file.clone();
    let new = old.replace(".epub", ".azw3");
    std::fs::rename(lib.root.join(&old), lib.root.join(&new)).unwrap();
    lib.get_mut(b).unwrap().file = new;
    lib.get_mut(b).unwrap().progress = 0.5;
    lib.save().unwrap();
    store.put_many(&lib.books).unwrap();
    let coll = store.create_collection("Shelf").unwrap();
    store.add_to_collection(coll, b).unwrap();
    store.set_status(b, ReadingStatus::OnHold).unwrap();

    // a refused merge (two EPUBs) changes nothing
    store.set_status(c, ReadingStatus::Finished).unwrap();
    assert!(bookx_store::formats::merge(&mut lib, &store, a, &[c]).is_err());
    assert_eq!(store.get_status(a), Some(ReadingStatus::New));

    assert_eq!(
        bookx_store::formats::merge(&mut lib, &store, a, &[b]).unwrap(),
        1
    );
    assert_eq!(lib.get(a).unwrap().formats.len(), 1);
    assert_eq!(lib.get(a).unwrap().progress, 0.5);
    assert!(store.collections_of(a).contains(&coll));
    assert_eq!(store.get_status(a), Some(ReadingStatus::OnHold));
    lib.save().unwrap();
    store.put_many(&lib.books).unwrap();
    assert_eq!(
        store.formats_of(a).len(),
        2,
        "the format rows follow the merged book"
    );
}
