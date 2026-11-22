//! The library catalogue in SQLite: opening, persistence, migration from the old JSON file,
//! incremental saves, the watcher used by the sharing server.

use std::path::{Path, PathBuf};

use bookx_core::convert::{Chapter, create_epub};
use bookx_core::library::{AddOutcome, Bookmark, Highlight, ReadPos};
use bookx_core::opf::Metadata;
use bookx_store::{
    CatalogWatcher, LEGACY_JSON, LIBRARY_DB, SqliteStore, has_library, library_db_path,
    open_library,
};
use chrono::Utc;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bookx-catalog-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn epub(dir: &Path, title: &str, author: &str) -> PathBuf {
    let md = Metadata {
        title: title.into(),
        authors: vec![author.into()],
        ..Default::default()
    };
    let ch = Chapter {
        title: "One".into(),
        body: format!("<p>Text of {title}.</p>"),
    };
    let p = dir.join(format!("{title}.epub"));
    std::fs::write(&p, create_epub(&md, &[ch], None).to_bytes().unwrap()).unwrap();
    p
}

fn add(lib: &mut bookx_core::library::Library, p: &Path) -> u64 {
    match lib.add_path(p, false).unwrap() {
        AddOutcome::Added(id) => id,
        AddOutcome::Duplicate(id) => id,
    }
}

#[test]
fn a_new_library_is_an_empty_database() {
    let root = tmp("new").join("lib");
    assert!(!has_library(&root));
    let o = open_library(&root).unwrap();
    assert!(o.library.books.is_empty());
    assert!(o.migrated.is_none());
    assert!(library_db_path(&root).exists());
    assert!(has_library(&root));
    assert_eq!(library_db_path(&root).file_name().unwrap(), LIBRARY_DB);
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn books_and_everything_attached_survive_a_reopen() {
    let t = tmp("persist");
    let root = t.join("lib");
    let id;
    {
        let mut o = open_library(&root).unwrap();
        id = add(&mut o.library, &epub(&t, "Persistent", "Ann Author"));
        let b = o.library.get_mut(id).unwrap();
        b.progress = 0.42;
        b.position = Some(ReadPos {
            spine: 3,
            frac: 0.5,
        });
        b.bookmarks.push(Bookmark {
            id: "bm".into(),
            spine: 1,
            frac: 0.1,
            label: "Mark".into(),
            created: Utc::now(),
        });
        b.highlights.push(Highlight {
            id: "hl".into(),
            spine: 2,
            start: 4,
            end: 9,
            color: "green".into(),
            style: "squiggly".into(),
            note: "a note \u{1F600}".into(),
            text: "quote".into(),
            created: Utc::now(),
        });
        o.library.save().unwrap();
    }
    let o = open_library(&root).unwrap();
    assert_eq!(o.library.books.len(), 1);
    let b = o.library.get(id).unwrap();
    assert_eq!(b.meta.title, "Persistent");
    assert_eq!(b.progress, 0.42);
    assert_eq!(b.position.unwrap().spine, 3);
    assert_eq!(b.bookmarks[0].label, "Mark");
    assert_eq!(b.highlights[0].style, "squiggly");
    assert_eq!(b.highlights[0].note, "a note \u{1F600}");
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn edits_removals_and_the_id_counter_persist() {
    let t = tmp("edits");
    let root = t.join("lib");
    let (a, b);
    {
        let mut o = open_library(&root).unwrap();
        a = add(&mut o.library, &epub(&t, "Alpha", "Ann"));
        b = add(&mut o.library, &epub(&t, "Beta", "Bob"));
        o.library.save().unwrap();
        o.library.get_mut(a).unwrap().progress = 1.0;
        o.library.remove(&[b], true).unwrap();
    }
    {
        let mut o = open_library(&root).unwrap();
        assert_eq!(o.library.books.len(), 1);
        assert_eq!(o.library.get(a).unwrap().progress, 1.0);
        assert!(o.library.get(b).is_none());
        // the removed id is not handed out again
        let c = add(&mut o.library, &epub(&t, "Gamma", "Cy"));
        assert!(c > b, "{c} should be above {b}");
        // the search index follows removals and additions
        assert!(o.store.search_ids("beta").is_empty());
        assert_eq!(o.store.search_ids("gamma"), vec![c]);
    }
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn saving_without_changes_writes_nothing() {
    let t = tmp("nochange");
    let root = t.join("lib");
    let mut o = open_library(&root).unwrap();
    add(&mut o.library, &epub(&t, "Quiet", "Q"));
    o.library.save().unwrap();
    let other = SqliteStore::open(&library_db_path(&root)).unwrap();
    let _ = other.data_version();
    // no change: the other connection sees no commit
    o.library.save().unwrap();
    o.library.save().unwrap();
    let before = other.data_version();
    o.library.save().unwrap();
    assert_eq!(other.data_version(), before);
    // a change is seen
    o.library.books[0].progress = 0.1;
    o.library.save().unwrap();
    assert_ne!(other.data_version(), before);
    let _ = std::fs::remove_dir_all(t);
}

// -- migration ----------------------------------------------------------------------------------

/// Builds a legacy `bookx-library.json` by exporting a database with two books.
fn legacy_json(t: &Path) -> (String, Vec<u64>) {
    let src = t.join("src-lib");
    let mut o = open_library(&src).unwrap();
    let a = add(&mut o.library, &epub(t, "Legacy One", "Old Author"));
    let b = add(&mut o.library, &epub(t, "Legacy Two", "Other Author"));
    o.library.get_mut(a).unwrap().progress = 0.3;
    o.library.save().unwrap();
    (o.store.export_catalog_json().unwrap(), vec![a, b])
}

#[test]
fn an_old_json_catalogue_is_moved_into_the_database_once() {
    let t = tmp("migrate");
    let (json, ids) = legacy_json(&t);
    let root = t.join("old-lib");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(LEGACY_JSON), &json).unwrap();
    assert!(has_library(&root));

    let o = open_library(&root).unwrap();
    assert_eq!(o.migrated, Some(2));
    assert_eq!(o.library.books.len(), 2);
    assert_eq!(o.library.get(ids[0]).unwrap().meta.title, "Legacy One");
    assert_eq!(o.library.get(ids[0]).unwrap().progress, 0.3);
    assert!(
        !root.join(LEGACY_JSON).exists(),
        "the old file is set aside"
    );
    assert!(root.join(format!("{LEGACY_JSON}.migrated")).exists());
    drop(o);

    // opening again does not migrate again
    let o = open_library(&root).unwrap();
    assert_eq!(o.migrated, None);
    assert_eq!(o.library.books.len(), 2);
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn a_json_file_next_to_a_filled_database_is_left_alone() {
    let t = tmp("keepjson");
    let (json, _) = legacy_json(&t);
    let root = t.join("lib");
    let mut o = open_library(&root).unwrap();
    add(&mut o.library, &epub(&t, "Already Here", "Me"));
    o.library.save().unwrap();
    drop(o);
    std::fs::write(root.join(LEGACY_JSON), json).unwrap();
    let o = open_library(&root).unwrap();
    assert_eq!(o.migrated, None);
    assert_eq!(o.library.books.len(), 1, "the stale file was not imported");
    assert!(root.join(LEGACY_JSON).exists());
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn a_damaged_old_catalogue_is_an_error_and_is_not_lost() {
    let t = tmp("damaged");
    let root = t.join("lib");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(LEGACY_JSON), "{ this is not json").unwrap();
    let err = open_library(&root).err().expect("must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains(LEGACY_JSON), "{msg}");
    assert!(
        root.join(LEGACY_JSON).exists(),
        "the file stays for the user to fix"
    );
    // after fixing it, the library opens
    let (json, _) = legacy_json(&t);
    std::fs::write(root.join(LEGACY_JSON), json).unwrap();
    assert_eq!(open_library(&root).unwrap().library.books.len(), 2);
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn migration_keeps_the_id_counter_so_ids_are_never_reused() {
    let t = tmp("counter");
    let (json, ids) = legacy_json(&t);
    let root = t.join("lib");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(LEGACY_JSON), json).unwrap();
    let mut o = open_library(&root).unwrap();
    let c = add(&mut o.library, &epub(&t, "Newcomer", "N"));
    assert!(c > *ids.iter().max().unwrap());
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn an_empty_legacy_catalogue_migrates_to_an_empty_library() {
    let t = tmp("emptyjson");
    let root = t.join("lib");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(LEGACY_JSON), r#"{"next_id": 5, "books": []}"#).unwrap();
    let o = open_library(&root).unwrap();
    assert_eq!(o.migrated, Some(0));
    assert!(o.library.books.is_empty());
    assert!(!root.join(LEGACY_JSON).exists());
    let _ = std::fs::remove_dir_all(t);
}

// -- export, stamps, watcher ----------------------------------------------------------------------

#[test]
fn the_json_export_round_trips() {
    let t = tmp("export");
    let (json, ids) = legacy_json(&t);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["books"].as_array().unwrap().len(), 2);
    assert!(v["next_id"].as_u64().unwrap() > ids[1]);
    // importing the export into a fresh database gives the same books
    let path = t.join("export.json");
    std::fs::write(&path, &json).unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    assert_eq!(s.import_json_catalog(&path).unwrap().imported, 2);
    assert_eq!(s.export_catalog_json().unwrap(), json);
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn text_stamps_follow_the_book() {
    let t = tmp("stamps");
    let root = t.join("lib");
    let mut o = open_library(&root).unwrap();
    let id = add(&mut o.library, &epub(&t, "Stamped", "S"));
    o.library.save().unwrap();
    assert!(o.store.text_stamps().is_empty());
    o.store
        .index_book_text_stamped(id, 111, "some body text")
        .unwrap();
    assert_eq!(o.store.text_stamps().get(&id), Some(&111));
    o.store
        .index_book_text_stamped(id, 222, "newer body text")
        .unwrap();
    assert_eq!(o.store.text_stamps().get(&id), Some(&222));
    assert_eq!(o.store.search_text("newer", 5).unwrap().len(), 1);
    assert!(
        o.store.search_text("some", 5).unwrap().is_empty(),
        "the old text was replaced"
    );
    // removing the book removes its text and stamp
    o.library.remove(&[id], true).unwrap();
    assert!(o.store.text_stamps().is_empty());
    assert!(o.store.search_text("newer", 5).unwrap().is_empty());
    // a book that does not exist cannot be indexed
    assert!(o.store.index_book_text_stamped(9999, 1, "x").is_err());
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn the_watcher_follows_changes_made_by_another_connection() {
    let t = tmp("watch");
    let root = t.join("lib");
    let w = CatalogWatcher::new(&root);
    assert!(w.books().is_empty(), "no database yet");
    let mut o = open_library(&root).unwrap();
    assert!(w.books().is_empty());
    add(&mut o.library, &epub(&t, "Watched", "W"));
    o.library.save().unwrap();
    assert_eq!(w.books().len(), 1);
    assert_eq!(w.books().len(), 1, "second call is served from the cache");
    add(&mut o.library, &epub(&t, "Watched Two", "W"));
    o.library.save().unwrap();
    assert_eq!(w.books().len(), 2);
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn the_catalogue_commit_is_atomic() {
    let t = tmp("atomic");
    let root = t.join("lib");
    let mut o = open_library(&root).unwrap();
    let id = add(&mut o.library, &epub(&t, "Atomic", "A"));
    o.library.save().unwrap();
    let book = o.library.get(id).unwrap().clone();
    // a commit whose upsert cannot be stored (id beyond i64) rolls everything back, including
    // the removal listed before it
    let mut bad = book.clone();
    bad.id = u64::MAX;
    let err = o.store.commit_catalog(u64::MAX, &[&bad], &[id]);
    assert!(err.is_err());
    assert!(o.store.contains(id), "the removal was rolled back");
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn many_libraries_in_one_process_do_not_interfere() {
    let t = tmp("many");
    std::thread::scope(|s| {
        for i in 0..6 {
            let t = t.clone();
            s.spawn(move || {
                let root = t.join(format!("lib{i}"));
                let mut o = open_library(&root).unwrap();
                for j in 0..3 {
                    add(&mut o.library, &epub(&t, &format!("Book {i}-{j}"), "Many"));
                }
                o.library.save().unwrap();
                assert_eq!(open_library(&root).unwrap().library.books.len(), 3);
            });
        }
    });
    let _ = std::fs::remove_dir_all(t);
}
