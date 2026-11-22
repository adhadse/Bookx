mod common;

use std::cell::Cell;
use std::path::Path;

use bookx_core::library::Book;
use bookx_core::opf::Identifier;
use bookx_store::{export_calibre, SqliteStore};
use common::*;
use rusqlite::Connection;

fn lib_book(root: &Path, id: u64, title: &str, author: &str) -> Book {
    let mut b = book(id, title, author);
    b.file = format!("books/b{id}.epub");
    std::fs::create_dir_all(root.join("books")).unwrap();
    std::fs::write(root.join(&b.file), format!("epub bytes {id}")).unwrap();
    b
}

fn sample(root: &Path) -> Vec<Book> {
    let mut dune = lib_book(root, 1, "Dune", "Frank Herbert");
    dune.meta.title_sort = "Dune".into();
    dune.meta.author_sort = "Herbert, Frank".into();
    dune.meta.tags = vec!["Science Fiction".into(), "Classic".into()];
    dune.meta.series = "Dune Chronicles".into();
    dune.meta.series_index = 1.0;
    dune.meta.publisher = "Chilton Books".into();
    dune.meta.description = "<div>A desert planet.</div>".into();
    dune.meta.language = "en".into();
    dune.meta.published = "1965-08-01".into();
    dune.meta.rating = 8;
    dune.meta.identifiers = vec![
        Identifier {
            scheme: "isbn".into(),
            value: "9780441013593".into(),
            xml_id: None,
        },
        Identifier {
            scheme: "goodreads".into(),
            value: "234225".into(),
            xml_id: None,
        },
    ];
    std::fs::write(root.join("books/dune.jpg"), b"jpegbytes").unwrap();
    dune.cover = Some("books/dune.jpg".into());
    // a second format
    std::fs::write(root.join("books/b1.mobi"), b"mobi").unwrap();
    dune.formats = vec![bookx_core::formats::FormatFile {
        file: "books/b1.mobi".into(),
        format: "mobi".into(),
        size: 4,
    }];

    let mut hobbit = lib_book(root, 2, "The Hobbit", "J. R. R. Tolkien");
    hobbit.meta.title_sort = "Hobbit, The".into();
    hobbit.meta.author_sort = "Tolkien, J. R. R.".into();

    let mut two = lib_book(root, 3, "Two Authors", "Smith, Jo");
    two.meta.authors = vec!["Smith, Jo".into(), "Second Author".into()];
    two.meta.author_sort = "Smith, Jo & Author, Second".into();

    vec![dune, hobbit, two]
}

fn never() -> bool {
    false
}

#[test]
fn round_trip_keeps_the_metadata() {
    let src = Tmp::new();
    let out = Tmp::new();
    let books = sample(src.path());
    let target = out.join("cal");
    let mut last = (0, 0);
    let r = export_calibre(
        &books,
        src.path(),
        &target,
        &mut |d, n| last = (d, n),
        &never,
    )
    .unwrap();
    assert_eq!((r.books, r.files, r.covers), (3, 4, 1), "{r:?}");
    assert!(r.problems.is_empty());
    assert_eq!(last, (3, 3));

    // layout
    assert!(target.join("metadata.db").is_file());
    let dir = target.join("Frank Herbert/Dune (1)");
    assert_eq!(
        std::fs::read(dir.join("Dune - Frank Herbert.epub")).unwrap(),
        b"epub bytes 1"
    );
    assert!(dir.join("Dune - Frank Herbert.mobi").is_file());
    assert_eq!(std::fs::read(dir.join("cover.jpg")).unwrap(), b"jpegbytes");
    let opf = std::fs::read_to_string(dir.join("metadata.opf")).unwrap();
    assert!(opf.contains("<dc:title>Dune</dc:title>") && opf.contains("calibre:series"));
    assert!(opf.contains("opf:scheme=\"ISBN\">9780441013593"));

    // the database is a valid SQLite file in Calibre's shape
    let c = Connection::open(target.join("metadata.db")).unwrap();
    let v: i32 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, bookx_store::CALIBRE_USER_VERSION);
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    let lib_id: i64 = c
        .query_row("SELECT count(*) FROM library_id", [], |r| r.get(0))
        .unwrap();
    assert_eq!(lib_id, 1);
    let triggers: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='trigger'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(triggers >= 40);
    let path: String = c
        .query_row("SELECT path FROM books WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(path, "Frank Herbert/Dune (1)");
    let lang: String = c
        .query_row("SELECT lang_code FROM languages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(lang, "eng");

    // and Bookx reads it back
    let back = SqliteStore::open_in_memory().unwrap();
    let rep = back.import_calibre(&target).unwrap();
    assert_eq!(rep.imported, 3);
    let imported = back.all_books().unwrap();
    for orig in &books {
        let got = imported
            .iter()
            .find(|b| b.meta.title == orig.meta.title)
            .unwrap_or_else(|| panic!("{} not imported", orig.meta.title));
        let (a, b) = (&orig.meta, &got.meta);
        assert_eq!(a.authors, b.authors, "{}", a.title);
        assert_eq!(a.author_sort, b.author_sort);
        assert_eq!(a.title_sort, b.title_sort);
        let (mut t1, mut t2) = (a.tags.clone(), b.tags.clone());
        t1.sort();
        t2.sort();
        assert_eq!(t1, t2);
        assert_eq!(a.series, b.series);
        assert_eq!(a.publisher, b.publisher);
        assert_eq!(a.description, b.description);
        assert_eq!(a.published, b.published);
        assert_eq!(a.rating, b.rating);
        assert_eq!(
            bookx_store::language_to_calibre(&a.language),
            b.language,
            "language"
        );
        let ids = |m: &bookx_core::opf::Metadata| {
            m.identifiers
                .iter()
                .map(|i| (i.scheme.clone(), i.value.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(a), ids(b));
        assert_eq!(orig.added.timestamp(), got.added.timestamp());
        assert_eq!(orig.modified.timestamp(), got.modified.timestamp());
        assert_eq!(got.cover.is_some(), orig.cover.is_some());
    }
}

#[test]
fn the_source_library_is_not_touched_and_a_non_empty_target_is_refused() {
    let src = Tmp::new();
    let out = Tmp::new();
    let books = sample(src.path());
    let before = tree(src.path());
    // non-empty target
    std::fs::write(out.join("keep.txt"), "mine").unwrap();
    let e = export_calibre(&books, src.path(), out.path(), &mut |_, _| {}, &never).unwrap_err();
    assert!(format!("{e:#}").contains("not empty"), "{e:#}");
    assert_eq!(
        std::fs::read_to_string(out.join("keep.txt")).unwrap(),
        "mine"
    );
    // a file as target
    let e = export_calibre(
        &books,
        src.path(),
        &out.join("keep.txt"),
        &mut |_, _| {},
        &never,
    )
    .unwrap_err();
    assert!(format!("{e:#}").contains("not a folder") || format!("{e:#}").contains("not empty"));
    // inside the library, or around it
    let inside = src.join("export");
    let e = export_calibre(&books, src.path(), &inside, &mut |_, _| {}, &never).unwrap_err();
    assert!(format!("{e:#}").contains("inside"), "{e:#}");
    assert!(!inside.exists());
    let e = export_calibre(&books, src.path(), src.path(), &mut |_, _| {}, &never).unwrap_err();
    assert!(format!("{e:#}").contains("inside") || format!("{e:#}").contains("not empty"));
    let parent = src.path().parent().unwrap();
    assert!(bookx_store::check_calibre_export_target(src.path(), parent).is_err());
    assert_eq!(before, tree(src.path()), "the source library changed");
    // an empty existing folder is fine
    let empty = out.join("empty");
    std::fs::create_dir(&empty).unwrap();
    export_calibre(&books, src.path(), &empty, &mut |_, _| {}, &never).unwrap();
}

fn tree(p: &Path) -> Vec<(String, u64)> {
    let mut v = Vec::new();
    fn walk(base: &Path, p: &Path, v: &mut Vec<(String, u64)>) {
        for e in std::fs::read_dir(p).unwrap().flatten() {
            let m = e.metadata().unwrap();
            let rel = e.path().strip_prefix(base).unwrap().display().to_string();
            v.push((rel, if m.is_file() { m.len() } else { 0 }));
            if m.is_dir() {
                walk(base, &e.path(), v);
            }
        }
    }
    walk(p, p, &mut v);
    v.sort();
    v
}

#[test]
fn hostile_names_stay_inside_the_target() {
    let src = Tmp::new();
    let out = Tmp::new();
    let mut b = lib_book(src.path(), 9, "../../escape", "../../../evil/author");
    b.meta.tags = vec!["a/b".into(), "../x".into()];
    // a book file path that points outside the library is only read, never used as a name
    let target = out.join("cal");
    let r = export_calibre(&[b], src.path(), &target, &mut |_, _| {}, &never).unwrap();
    assert_eq!(r.books, 1);
    for (rel, _) in tree(&target) {
        assert!(
            Path::new(&rel)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "{rel}"
        );
    }
    assert_eq!(
        tree(out.path())
            .iter()
            .filter(|(p, _)| !p.starts_with("cal"))
            .count(),
        0
    );
}

#[test]
fn cancelling_removes_everything() {
    let src = Tmp::new();
    let out = Tmp::new();
    let books = sample(src.path());
    let asked = Cell::new(0);
    let target = out.join("cal");
    let r = export_calibre(&books, src.path(), &target, &mut |_, _| {}, &|| {
        asked.set(asked.get() + 1);
        asked.get() > 1
    })
    .unwrap();
    assert!(r.cancelled);
    assert!(
        !target.exists(),
        "a folder created by the export is removed"
    );
    // an existing empty folder stays, but empty
    let keep = out.join("keep");
    std::fs::create_dir(&keep).unwrap();
    asked.set(0);
    let r = export_calibre(&books, src.path(), &keep, &mut |_, _| {}, &|| {
        asked.set(asked.get() + 1);
        asked.get() > 2
    })
    .unwrap();
    assert!(r.cancelled);
    assert!(keep.is_dir() && std::fs::read_dir(&keep).unwrap().next().is_none());
}

#[test]
fn missing_and_irregular_files_are_reported_not_fatal() {
    let src = Tmp::new();
    let out = Tmp::new();
    let mut books = sample(src.path());
    std::fs::remove_file(src.join("books/b2.epub")).unwrap();
    // a directory where a file should be
    std::fs::remove_file(src.join("books/b3.epub")).unwrap();
    std::fs::create_dir(src.join("books/b3.epub")).unwrap();
    books[0].cover = Some("books/missing.jpg".into());
    let target = out.join("cal");
    let r = export_calibre(&books, src.path(), &target, &mut |_, _| {}, &never).unwrap();
    assert_eq!(r.books, 3);
    assert_eq!(r.problems.len(), 2, "{:?}", r.problems);
    assert_eq!(r.covers, 0);
    let c = Connection::open(target.join("metadata.db")).unwrap();
    let n: i64 = c
        .query_row("SELECT count(*) FROM data", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
    let hc: i64 = c
        .query_row("SELECT has_cover FROM books WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hc, 0);
}

#[test]
fn duplicate_names_and_empty_fields_are_valid() {
    let src = Tmp::new();
    let out = Tmp::new();
    let mut a = lib_book(src.path(), 1, "Same", "Ann");
    let mut b = lib_book(src.path(), 2, "Same", "ann");
    a.meta.tags = vec!["x".into(), "X".into(), "x, y".into()];
    b.meta.authors = vec![];
    b.meta.author_sort = String::new();
    b.meta.title_sort = String::new();
    let target = out.join("cal");
    export_calibre(&[a, b], src.path(), &target, &mut |_, _| {}, &never).unwrap();
    let c = Connection::open(target.join("metadata.db")).unwrap();
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    let tags: i64 = c
        .query_row("SELECT count(*) FROM tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tags, 2, "x and X are one tag; 'x, y' becomes 'x; y'");
}

/// Calibre itself (when installed) opens the export. Skipped without `calibredb`.
#[test]
fn calibredb_opens_the_export() {
    let Ok(out) = std::process::Command::new("calibredb")
        .arg("--version")
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let src = Tmp::new();
    let dst = Tmp::new();
    let books = sample(src.path());
    let target = dst.join("cal");
    export_calibre(&books, src.path(), &target, &mut |_, _| {}, &never).unwrap();
    // calibredb needs a writable HOME/config; keep it in the temp folder
    let run = |args: &[&str]| {
        std::process::Command::new("calibredb")
            .args(args)
            .arg("--with-library")
            .arg(&target)
            .env("HOME", dst.path())
            .env("XDG_CONFIG_HOME", dst.join("config"))
            .env("XDG_CACHE_HOME", dst.join("cache"))
            .env("QT_QPA_PLATFORM", "offscreen")
            .output()
            .unwrap()
    };
    let l = run(&[
        "list",
        "--for-machine",
        "--fields",
        "title,authors,tags,series,publisher,rating,formats,identifiers",
    ]);
    let text = String::from_utf8_lossy(&l.stdout).to_string();
    assert!(
        l.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&l.stderr)
    );
    // calibre 9 prints "Upgrading database to version N..." after the JSON when it migrates an older database
    let rows: serde_json::Value = serde_json::Deserializer::from_str(&text)
        .into_iter::<serde_json::Value>()
        .next()
        .expect("calibredb printed JSON")
        .unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 3, "{text}");
    let dune = rows
        .iter()
        .find(|r| r["title"] == "Dune")
        .expect("Dune listed");
    assert_eq!(dune["authors"], "Frank Herbert");
    assert_eq!(dune["series"], "Dune Chronicles");
    assert_eq!(dune["publisher"], "Chilton Books");
    assert_eq!(dune["rating"], 8);
    assert_eq!(dune["identifiers"]["isbn"], "9780441013593");
    assert_eq!(dune["formats"].as_array().unwrap().len(), 2);
    assert_eq!(dune["tags"].as_array().unwrap().len(), 2);
    let two = rows.iter().find(|r| r["title"] == "Two Authors").unwrap();
    assert!(two["authors"].as_str().unwrap().contains("Second Author"));
    // a second pass: Calibre's own consistency check accepts the library
    let c = run(&["check_library"]);
    assert!(c.status.success(), "{}", String::from_utf8_lossy(&c.stdout));
    // and Calibre can edit it: its triggers (sort, uuid) work on the exported schema
    let extra = dst.join("extra.txt");
    std::fs::write(&extra, "plain text book").unwrap();
    let a = run(&[
        "add",
        "--title",
        "Added In Calibre",
        "--authors",
        "New Author",
        extra.to_str().unwrap(),
    ]);
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let l = run(&["list", "--for-machine", "--fields", "title,uuid"]);
    let rows: serde_json::Value = serde_json::from_slice(&l.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 4);
    let added = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == "Added In Calibre")
        .unwrap();
    assert_eq!(added["uuid"].as_str().unwrap().len(), 36);
}

fn custom_defs() -> Vec<bookx_core::custom::ColumnDef> {
    use bookx_core::custom::{ColumnDef, ColumnKind};
    let c = |label: &str, kind| ColumnDef {
        label: label.into(),
        name: label.to_uppercase(),
        kind,
        ..Default::default()
    };
    let mut mood = c("mood", ColumnKind::Enumeration);
    mood.values = vec!["Calm".into(), "Tense".into()];
    vec![
        c("pages", ColumnKind::Integer),
        c("score", ColumnKind::Rating),
        c("done", ColumnKind::Bool),
        c("since", ColumnKind::Date),
        c("price", ColumnKind::Float),
        c("labels", ColumnKind::Tags),
        c("note", ColumnKind::Text),
        c("review", ColumnKind::LongText),
        mood,
    ]
}

#[test]
fn custom_columns_go_to_calibre_and_come_back() {
    let src = Tmp::new();
    let out = Tmp::new();
    let mut books = sample(src.path());
    for (k, v) in [
        ("pages", "412"),
        ("score", "3.5"),
        ("done", "true"),
        ("since", "2024-05-17"),
        ("price", "9.5"),
        ("labels", "alpha, beta"),
        ("note", "lent to Sam"),
        ("review", "Line one\nLine two"),
        ("mood", "Tense"),
    ] {
        books[0].custom.insert(k.into(), v.into());
    }
    books[1].custom.insert("pages".into(), "100".into());
    let target = out.join("cal");
    bookx_store::export_calibre_with_columns(
        &books,
        &custom_defs(),
        src.path(),
        &target,
        &mut |_, _| {},
        &never,
    )
    .unwrap();

    // Calibre's tables
    let c = Connection::open(target.join("metadata.db")).unwrap();
    let n: i64 = c
        .query_row("SELECT count(*) FROM custom_columns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 9);
    let pages: i64 = c
        .query_row(
            "SELECT value FROM custom_column_1 WHERE book = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pages, 412);
    let score: i64 = c
        .query_row(
            "SELECT v.value FROM custom_column_2 v JOIN books_custom_column_2_link l ON l.value = v.id WHERE l.book = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(score, 7, "Calibre rates 0 to 10");
    let tags: i64 = c
        .query_row(
            "SELECT count(*) FROM books_custom_column_6_link WHERE book = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tags, 2);
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    drop(c);

    // Calibre itself lists the columns and the values (skipped when calibredb is not installed)
    if let Ok(o) = std::process::Command::new("calibredb")
        .args(["list", "--with-library"])
        .arg(&target)
        .args(["--fields", "title,*pages,*mood", "--for-machine"])
        .env("HOME", out.path())
        .output()
    {
        if !o.status.success() {
            eprintln!("calibredb: {}", String::from_utf8_lossy(&o.stderr));
        }
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout);
            assert!(text.contains("412") && text.contains("Tense"), "{text}");
        }
    }

    // and back into a fresh store
    let store = SqliteStore::open_in_memory().unwrap();
    let imp = store
        .import_calibre_with(&target, false, &mut |_, _| {})
        .unwrap();
    assert_eq!(imp.custom_columns.len(), 9, "{imp:?}");
    let cols = store.custom_columns();
    assert_eq!(cols.len(), 9);
    assert_eq!(
        cols.iter().find(|c| c.label == "mood").unwrap().values,
        vec!["Calm", "Tense"]
    );
    let back = store.all_books().unwrap();
    let dune = back.iter().find(|b| b.meta.title == "Dune").unwrap();
    for (k, v) in [
        ("pages", "412"),
        ("score", "3.5"),
        ("done", "true"),
        ("since", "2024-05-17"),
        ("price", "9.5"),
        ("labels", "alpha, beta"),
        ("note", "lent to Sam"),
        ("review", "Line one\nLine two"),
        ("mood", "Tense"),
    ] {
        assert_eq!(dune.custom_value(k), v, "{k}");
    }
    let hobbit = back.iter().find(|b| b.meta.title == "The Hobbit").unwrap();
    assert_eq!(hobbit.custom_value("pages"), "100");
    assert_eq!(hobbit.custom_value("mood"), "");
    assert_eq!(store.search_ids("#pages:>200"), vec![dune.id]);
}
