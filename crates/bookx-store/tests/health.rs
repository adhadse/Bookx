//! Snapshots, restore, the library check, the lock, and the Calibre import with copying
//! (roadmap PP-C-1, PP-C-2, PP-C-3, PP-C-16, PP-G-2).

mod common;

use std::path::Path;

use bookx_core::layout::Layout;
use bookx_core::library::{AddOutcome, Library};
use bookx_store::health::*;
use bookx_store::{open_library, OpenedLibrary};
use chrono::{Duration, TimeZone, Utc};
use common::Tmp;

fn md(dir: &Path, title: &str, author: &str) -> std::path::PathBuf {
    let p = dir.join(format!("{title}.md"));
    std::fs::write(
        &p,
        format!("---\ntitle: \"{title}\"\nauthor: \"{author}\"\n---\n# One\n\nHello {title}.\n"),
    )
    .unwrap();
    p
}

fn add(lib: &mut Library, dir: &Path, title: &str, author: &str) -> u64 {
    match lib.add_path(&md(dir, title, author), false).unwrap() {
        AddOutcome::Added(id) => id,
        _ => panic!("duplicate"),
    }
}

fn open(t: &Tmp) -> OpenedLibrary {
    open_library(&t.join("lib")).unwrap()
}

#[test]
fn a_snapshot_is_listed_restored_and_the_restore_is_itself_undoable() {
    let t = Tmp::new();
    let mut o = open(&t);
    add(&mut o.library, t.path(), "Dune", "Frank Herbert");
    let now = Utc::now();
    let snap = take_snapshot(&o.store, &o.library.root, SnapshotKind::Manual, now).unwrap();
    assert_eq!(snap.books, 1);
    assert!(snap.path.exists());

    add(&mut o.library, t.path(), "Emma", "Jane Austen");
    assert_eq!(o.store.len(), 2);

    restore_snapshot(
        &o.store,
        &o.library.root,
        &snap.path,
        now + Duration::seconds(5),
    )
    .unwrap();
    assert_eq!(o.store.len(), 1);
    let snaps = list_snapshots(&o.library.root);
    assert_eq!(snaps.len(), 2);
    let pre = snaps
        .iter()
        .find(|s| s.kind == SnapshotKind::PreRestore)
        .unwrap();
    assert_eq!(pre.books, 2, "the catalogue before the restore was kept");

    // and the restore can be taken back
    restore_snapshot(
        &o.store,
        &o.library.root,
        &pre.path,
        now + Duration::seconds(10),
    )
    .unwrap();
    assert_eq!(o.store.len(), 2);
}

#[test]
fn a_damaged_backup_is_refused_and_changes_nothing() {
    let t = Tmp::new();
    let mut o = open(&t);
    add(&mut o.library, t.path(), "Dune", "Frank Herbert");
    let bad = t.join("bad.db");
    std::fs::write(&bad, vec![7u8; 5000]).unwrap();
    assert!(restore_snapshot(&o.store, &o.library.root, &bad, Utc::now()).is_err());
    assert_eq!(o.store.len(), 1);
    assert!(
        list_snapshots(&o.library.root).is_empty(),
        "no safety snapshot for a refused restore"
    );
}

#[test]
fn automatic_snapshots_skip_an_unchanged_library_and_prune() {
    let t = Tmp::new();
    let mut o = open(&t);
    add(&mut o.library, t.path(), "Dune", "Frank Herbert");
    let root = o.library.root.clone();
    let t0 = Utc::now();
    let first = auto_snapshot(&o.store, &root, 2, t0).unwrap();
    assert!(first.is_some());
    // nothing changed since: no second snapshot
    let again = auto_snapshot(&o.store, &root, 2, t0 + Duration::seconds(30)).unwrap();
    assert!(again.is_none());
    assert_eq!(list_snapshots(&root).len(), 1);

    // many snapshots from the past are pruned down to keep + one per recent day
    for i in 1..=6 {
        take_snapshot(
            &o.store,
            &root,
            SnapshotKind::Auto,
            t0 - Duration::days(30 + i),
        )
        .unwrap();
    }
    assert_eq!(list_snapshots(&root).len(), 7);
    let removed = prune_snapshots(&root, 2, t0).unwrap();
    assert_eq!(
        removed, 5,
        "the newest 2 stay: the recent one and the newest old one"
    );
    assert_eq!(list_snapshots(&root).len(), 2);
}

#[test]
fn two_snapshots_in_one_second_do_not_overwrite_each_other() {
    let t = Tmp::new();
    let o = open(&t);
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let a = take_snapshot(&o.store, &o.library.root, SnapshotKind::Manual, now).unwrap();
    let b = take_snapshot(&o.store, &o.library.root, SnapshotKind::Manual, now).unwrap();
    assert_ne!(a.path, b.path);
}

#[test]
fn the_check_finds_missing_files_covers_orphans_and_fixes_them() {
    let t = Tmp::new();
    let mut o = open(&t);
    let a = add(&mut o.library, t.path(), "Dune", "Frank Herbert");
    let b = add(&mut o.library, t.path(), "Emma", "Jane Austen");
    let report = check_library(&o.library, Some(&o.store), CheckOptions::default());
    assert!(report.is_clean(), "{:?}", report.findings);
    assert_eq!(report.books_checked, 2);

    // Dune's file disappears, an unknown epub appears
    let path = o.library.book_path(o.library.get(a).unwrap());
    std::fs::remove_file(&path).unwrap();
    let stray = o.library.root.join("stray.epub");
    std::fs::copy(o.library.book_path(o.library.get(b).unwrap()), &stray).unwrap();
    let report = check_library(&o.library, Some(&o.store), CheckOptions::default());
    assert_eq!(report.errors(), 1);
    let missing = report
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::MissingFile)
        .unwrap();
    assert_eq!(missing.book, Some(a));
    assert!(missing.message.contains("Dune"));
    let orphan = report
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Orphan)
        .unwrap();
    assert!(orphan.message.contains("stray.epub"));

    apply_fix(&mut o.library, missing, &Fix::RemoveEntry).unwrap();
    assert!(o.library.get(a).is_none());
    assert_eq!(o.library.trash().len(), 1, "removing the entry is undoable");
    // the stray copy is the same book as Emma: reported as a duplicate, not added twice
    let msg = apply_fix(&mut o.library, orphan, &orphan.fixes[0]).unwrap();
    assert!(msg.contains("not added"), "{msg}");
}

#[test]
fn drift_between_file_and_catalogue_is_found_and_fixed_both_ways() {
    let t = Tmp::new();
    let mut o = open(&t);
    let id = add(&mut o.library, t.path(), "Dune", "Frank Herbert");
    // the catalogue changes without the file (an edit in the database only)
    o.library.get_mut(id).unwrap().meta.title = "Dune (catalogue edit)".into();
    let opts = CheckOptions { drift: true };
    let report = check_library(&o.library, None, opts);
    let f = report
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Drift)
        .unwrap();
    assert!(
        f.message.contains("Dune (catalogue edit)") && f.message.contains("title"),
        "{}",
        f.message
    );
    assert!(check_library(&o.library, None, CheckOptions::default()).is_clean());

    apply_fix(&mut o.library, f, &Fix::UseFile).unwrap();
    assert_eq!(o.library.get(id).unwrap().meta.title, "Dune");
    assert!(check_library(&o.library, None, opts).is_clean());

    o.library.get_mut(id).unwrap().meta.title = "Chosen Title".into();
    let report = check_library(&o.library, None, opts);
    apply_fix(&mut o.library, &report.findings[0], &Fix::UseCatalogue).unwrap();
    assert_eq!(
        o.library.open_book(id).unwrap().metadata().title,
        "Chosen Title"
    );
    assert!(check_library(&o.library, None, opts).is_clean());
}

#[test]
fn a_damaged_database_shows_up_in_the_check() {
    let t = Tmp::new();
    let mut o = open(&t);
    let id = add(&mut o.library, t.path(), "Dune", "Frank Herbert");
    // the search index loses a row behind the catalogue's back
    rusqlite::Connection::open(t.join("lib/bookx-library.db"))
        .unwrap()
        .execute("DELETE FROM fts_meta WHERE rowid = ?1", [id as i64])
        .unwrap();
    let report = check_library(&o.library, Some(&o.store), CheckOptions::default());
    let f = report
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::Database)
        .unwrap();
    assert_eq!(f.severity, Severity::Error);
    assert!(f.message.contains("Restore a backup"), "{}", f.message);
}

#[test]
fn the_read_only_second_opener_cannot_change_the_library() {
    let t = Tmp::new();
    let root = t.join("lib");
    let bookx_store::guard::Lock::Acquired(_first) =
        bookx_store::guard::acquire(&root, "Bookx").unwrap()
    else {
        panic!()
    };
    let bookx_store::guard::Lock::Busy(h) =
        bookx_store::guard::acquire(&root, "Bookx CLI").unwrap()
    else {
        panic!("the same live process holds the lock")
    };
    assert_eq!(h.pid, std::process::id());
    let mut o = open_library(&root).unwrap();
    o.library.set_read_only(true);
    assert!(o
        .library
        .add_path(&md(t.path(), "Dune", "Frank Herbert"), false)
        .is_err());
}

// -- Calibre import -------------------------------------------------------------------------------

fn calibre_fixture(dir: &Path) {
    let book_dir = dir.join("Frank Herbert/Dune (1)");
    std::fs::create_dir_all(&book_dir).unwrap();
    // a real EPUB made through a scratch library
    let scratch = Tmp::new();
    let mut l = Library::in_memory(&scratch.join("l")).unwrap();
    let id = add(&mut l, scratch.path(), "Dune", "Frank Herbert");
    l.export_copy(id, &book_dir.join("Dune - Frank Herbert.epub"))
        .unwrap();
    std::fs::write(book_dir.join("cover.jpg"), b"jpegdata").unwrap();
    let c = rusqlite::Connection::open(dir.join("metadata.db")).unwrap();
    c.execute_batch(
        "CREATE TABLE books(id INTEGER PRIMARY KEY, title TEXT, sort TEXT, timestamp TEXT, last_modified TEXT,
            pubdate TEXT, series_index REAL, author_sort TEXT, path TEXT, has_cover INTEGER);
         CREATE TABLE data(id INTEGER PRIMARY KEY, book INTEGER, format TEXT, uncompressed_size INTEGER, name TEXT);
         CREATE TABLE authors(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE books_authors_link(id INTEGER PRIMARY KEY, book INTEGER, author INTEGER);
         CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE books_tags_link(id INTEGER PRIMARY KEY, book INTEGER, tag INTEGER);
         CREATE TABLE series(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE books_series_link(id INTEGER PRIMARY KEY, book INTEGER, series INTEGER);
         CREATE TABLE publishers(id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE books_publishers_link(id INTEGER PRIMARY KEY, book INTEGER, publisher INTEGER);
         CREATE TABLE comments(id INTEGER PRIMARY KEY, book INTEGER, text TEXT);
         CREATE TABLE languages(id INTEGER PRIMARY KEY, lang_code TEXT);
         CREATE TABLE books_languages_link(id INTEGER PRIMARY KEY, book INTEGER, lang_code INTEGER, item_order INTEGER);
         CREATE TABLE ratings(id INTEGER PRIMARY KEY, rating INTEGER);
         CREATE TABLE books_ratings_link(id INTEGER PRIMARY KEY, book INTEGER, rating INTEGER);
         CREATE TABLE identifiers(id INTEGER PRIMARY KEY, book INTEGER, type TEXT, val TEXT);
         INSERT INTO books VALUES(1,'Dune','Dune','2020-01-01 00:00:00+00:00','2020-01-01 00:00:00+00:00',
            '1965-08-01 00:00:00+00:00',1.0,'Herbert, Frank','Frank Herbert/Dune (1)',1);
         INSERT INTO books VALUES(2,'No Epub','No Epub','2020-01-01 00:00:00+00:00','2020-01-01 00:00:00+00:00',
            '0101-01-01 00:00:00+00:00',1.0,'Nobody','Nobody/No Epub (2)',0);
         INSERT INTO data VALUES(1,1,'EPUB',100,'Dune - Frank Herbert');
         INSERT INTO data VALUES(2,2,'PDF',100,'No Epub');
         INSERT INTO authors VALUES(1,'Frank Herbert'),(2,'Nobody');
         INSERT INTO books_authors_link VALUES(1,1,1),(2,2,2);",
    )
    .unwrap();
}

#[test]
fn a_calibre_import_links_in_place_with_progress_and_a_report() {
    let t = Tmp::new();
    let cal = t.join("calibre");
    std::fs::create_dir_all(&cal).unwrap();
    calibre_fixture(&cal);
    let o = open(&t);
    let mut ticks = Vec::new();
    let r = o
        .store
        .import_calibre_with(&cal, true, &mut |d, n| ticks.push((d, n)))
        .unwrap();
    assert_eq!(r.report.imported, 1);
    assert_eq!(r.skipped, vec!["No Epub".to_string()]);
    assert_eq!(ticks.first(), Some(&(0, 2)));
    assert_eq!(ticks.last(), Some(&(2, 2)));
    // reload the library from the store: the linked book opens straight from the Calibre folder
    let lib = Library::with_backend(
        &o.library.root,
        Box::new(bookx_store::SqliteCatalog::new(o.store.clone())),
    )
    .unwrap();
    let b = lib.get(r.ids[0]).unwrap();
    assert!(Path::new(&b.file).is_absolute());
    assert_eq!(lib.open_book(b.id).unwrap().metadata().title, "Dune");
    // the import twice only reports the duplicate
    let again = o
        .store
        .import_calibre_with(&cal, true, &mut |_, _| {})
        .unwrap();
    assert_eq!(again.report.duplicates, 1);
    assert!(again.ids.is_empty());
}

#[test]
fn imported_calibre_books_can_be_copied_into_the_library_layout() {
    let t = Tmp::new();
    let cal = t.join("calibre");
    std::fs::create_dir_all(&cal).unwrap();
    calibre_fixture(&cal);
    let o = open(&t);
    let r = o
        .store
        .import_calibre_with(&cal, true, &mut |_, _| {})
        .unwrap();
    let mut lib = Library::with_backend(
        &o.library.root,
        Box::new(bookx_store::SqliteCatalog::new(o.store.clone())),
    )
    .unwrap();
    lib.set_layout(Layout::Flat).unwrap();
    for id in &r.ids {
        lib.copy_into_library(*id).unwrap();
    }
    lib.save().unwrap();
    let b = lib.get(r.ids[0]).unwrap();
    assert_eq!(b.file, "Frank Herbert - Dune.epub");
    assert!(lib.book_path(b).exists());
    assert!(
        cal.join("Frank Herbert/Dune (1)/Dune - Frank Herbert.epub")
            .exists(),
        "the Calibre files stay"
    );
    assert_eq!(b.cover.as_deref(), Some(".covers/1.jpg"));
    assert_eq!(
        std::fs::read(lib.cover_path(b).unwrap()).unwrap(),
        b"jpegdata"
    );
}

#[test]
fn the_one_call_import_copies_and_reports() {
    let t = Tmp::new();
    let cal = t.join("calibre");
    std::fs::create_dir_all(&cal).unwrap();
    calibre_fixture(&cal);
    let o = open(&t);
    let mut stages = Vec::new();
    let r =
        bookx_store::import_calibre_into(&o.store, &o.library.root, &cal, true, &mut |s, d, n| {
            stages.push((s, d, n))
        })
        .unwrap();
    assert!(stages.contains(&(bookx_store::Stage::Reading, 2, 2)));
    assert!(stages.contains(&(bookx_store::Stage::Copying, 1, 1)));
    assert!(r.copy_failed.is_empty());
    let lib = Library::with_backend(
        &o.library.root,
        Box::new(bookx_store::SqliteCatalog::new(o.store.clone())),
    )
    .unwrap();
    let b = lib.get(r.ids[0]).unwrap();
    assert!(!Path::new(&b.file).is_absolute(), "copied into the library");
    assert!(lib.book_path(b).exists());
    let text = r.summary();
    assert!(
        text.contains("1 book(s) imported") && text.contains("No Epub"),
        "{text}"
    );
}

#[test]
fn a_book_that_cannot_be_copied_stays_linked_and_is_reported() {
    let t = Tmp::new();
    let cal = t.join("calibre");
    std::fs::create_dir_all(&cal).unwrap();
    calibre_fixture(&cal);
    std::fs::remove_file(cal.join("Frank Herbert/Dune (1)/Dune - Frank Herbert.epub")).unwrap();
    let o = open(&t);
    let r =
        bookx_store::import_calibre_into(&o.store, &o.library.root, &cal, true, &mut |_, _, _| {})
            .unwrap();
    assert_eq!(r.report.imported, 1);
    assert_eq!(r.copy_failed.len(), 1);
    assert!(r.summary().contains("could not be copied"));
}
