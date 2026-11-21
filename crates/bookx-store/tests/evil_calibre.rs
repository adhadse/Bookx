//! Hostile `metadata.db` files: a broken or malicious Calibre library must produce an error or a partial import,
//! never a panic, a hang, or a link to a file outside the library folder.

mod common;

use bookx_store::SqliteStore;
use common::*;
use rusqlite::Connection;

const CALIBRE_SCHEMA: &str = "
CREATE TABLE books(id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL DEFAULT 'Unknown',
  sort TEXT, timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP, pubdate TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  series_index REAL NOT NULL DEFAULT 1.0, author_sort TEXT, isbn TEXT DEFAULT '', lccn TEXT DEFAULT '',
  path TEXT NOT NULL DEFAULT '', flags INTEGER NOT NULL DEFAULT 1, uuid TEXT, has_cover BOOL DEFAULT 0,
  last_modified TIMESTAMP NOT NULL DEFAULT '2000-01-01 00:00:00+00:00');
CREATE TABLE authors(id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort TEXT, link TEXT NOT NULL DEFAULT '');
CREATE TABLE books_authors_link(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, author INTEGER NOT NULL);
CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT NOT NULL, link TEXT NOT NULL DEFAULT '');
CREATE TABLE books_tags_link(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, tag INTEGER NOT NULL);
CREATE TABLE series(id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort TEXT, link TEXT NOT NULL DEFAULT '');
CREATE TABLE books_series_link(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, series INTEGER NOT NULL);
CREATE TABLE publishers(id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort TEXT, link TEXT NOT NULL DEFAULT '');
CREATE TABLE books_publishers_link(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, publisher INTEGER NOT NULL);
CREATE TABLE comments(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, text TEXT NOT NULL);
CREATE TABLE identifiers(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, type TEXT NOT NULL DEFAULT 'isbn', val TEXT NOT NULL);
CREATE TABLE ratings(id INTEGER PRIMARY KEY, rating INTEGER, link TEXT NOT NULL DEFAULT '');
CREATE TABLE books_ratings_link(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, rating INTEGER NOT NULL);
CREATE TABLE languages(id INTEGER PRIMARY KEY, lang_code TEXT NOT NULL, link TEXT NOT NULL DEFAULT '');
CREATE TABLE books_languages_link(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, lang_code INTEGER NOT NULL, item_order INTEGER NOT NULL DEFAULT 0);
CREATE TABLE data(id INTEGER PRIMARY KEY, book INTEGER NOT NULL, format TEXT NOT NULL, uncompressed_size INTEGER NOT NULL, name TEXT NOT NULL);
CREATE TABLE custom_columns(id INTEGER PRIMARY KEY, label TEXT NOT NULL, name TEXT NOT NULL, datatype TEXT NOT NULL);
INSERT INTO custom_columns VALUES(1, 'mood', 'Mood', 'text');
";

fn lib_with(t: &Tmp, rows: &str) {
    let c = Connection::open(t.join("metadata.db")).unwrap();
    c.execute_batch(CALIBRE_SCHEMA).unwrap();
    c.execute_batch(rows).unwrap();
}

#[test]
fn a_metadata_db_that_is_not_sqlite_is_an_error() {
    let t = Tmp::new();
    std::fs::write(t.join("metadata.db"), b"this is not a database at all").unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    assert!(s.import_calibre(t.path()).is_err());
    assert_eq!(s.len(), 0);
}

#[test]
fn a_truncated_database_is_an_error_or_a_partial_import() {
    let t = Tmp::new();
    lib_with(
        &t,
        "INSERT INTO books VALUES(1,'A','A',NULL,NULL,1.0,NULL,'','','A/A (1)',0,'u',0,'x');
         INSERT INTO data VALUES(1,1,'EPUB',1,'A');",
    );
    let bytes = std::fs::read(t.join("metadata.db")).unwrap();
    std::fs::write(t.join("metadata.db"), &bytes[..bytes.len() / 2]).unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    let _ = s.import_calibre(t.path());
}

#[test]
fn a_database_without_the_calibre_tables_is_an_error() {
    let t = Tmp::new();
    let c = Connection::open(t.join("metadata.db")).unwrap();
    c.execute_batch("CREATE TABLE unrelated(x)").unwrap();
    drop(c);
    let s = SqliteStore::open_in_memory().unwrap();
    assert!(s.import_calibre(t.path()).is_err());
}

#[test]
fn paths_that_climb_out_of_the_library_are_not_followed() {
    let t = Tmp::new();
    let outside = Tmp::new();
    std::fs::write(outside.join("secret.epub"), b"PK").unwrap();
    // `path` and the file name point at a file next to the library, not in it
    lib_with(
        &t,
        "INSERT INTO books VALUES(1,'Climb','Climb',NULL,NULL,1.0,NULL,'','','../../../../../../../../etc',0,'u',0,'x');
         INSERT INTO data VALUES(1,1,'EPUB',1,'passwd');
         INSERT INTO books VALUES(2,'Abs','Abs',NULL,NULL,1.0,NULL,'','','/etc',0,'u2',0,'x');
         INSERT INTO data VALUES(2,2,'EPUB',1,'passwd');",
    );
    let s = SqliteStore::open_in_memory().unwrap();
    let out = s
        .import_calibre_with(t.path(), true, &mut |_, _| {})
        .map(|o| o.report);
    // whatever happened, no imported book may name a file outside the library folder
    let root = t.path().canonicalize().unwrap();
    use bookx_core::contracts::Store;
    for id in 1..=2u64 {
        if let Some(b) = s.get(id) {
            let p = std::path::Path::new(&b.file);
            let full = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            };
            let real = full.canonicalize().unwrap_or(full);
            assert!(
                real.starts_with(&root),
                "book {id} points at {}",
                real.display()
            );
        }
    }
    let _ = out;
}

#[test]
fn absurd_text_values_are_imported_bounded_or_refused() {
    let t = Tmp::new();
    let huge = "x".repeat(2_000_000);
    lib_with(
        &t,
        &format!(
            "INSERT INTO books VALUES(1,'{huge}','T',NULL,NULL,1e308,'a','','','P/T (1)',0,'u',0,'x');
             INSERT INTO data VALUES(1,1,'EPUB',1,'T');"
        ),
    );
    let s = SqliteStore::open_in_memory().unwrap();
    let _ = s.import_calibre(t.path());
}
