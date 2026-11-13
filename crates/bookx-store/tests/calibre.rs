mod common;

use bookx_core::contracts::Store;
use bookx_store::{ImportReport, SqliteStore};
use common::*;
use rusqlite::Connection;

/// Real Calibre table and column names (metadata.db, schema as created by Calibre 6/7).
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

const FIXTURE: &str = "
INSERT INTO books VALUES(1,'Dune','Dune','2021-03-04 05:06:07.123456+00:00','1965-08-01 00:00:00+00:00',1.0,'Herbert, Frank','', '', 'Frank Herbert/Dune (1)',1,'u1',1,'2022-01-01 10:00:00+00:00');
INSERT INTO books VALUES(2,'The Hobbit','Hobbit, The','2020-05-06 07:08:09+00:00','0101-01-01 00:00:00+00:00',1.0,'Tolkien, J. R. R.','', '', 'J. R. R. Tolkien/The Hobbit (2)',0,'u2',0,'2020-05-06 07:08:09+00:00');
INSERT INTO books VALUES(3,'Only A PDF','Only A PDF','2020-01-01 00:00:00+00:00','0101-01-01 00:00:00+00:00',1.0,'Nobody','', '', 'Nobody/Only A PDF (3)',0,'u3',0,'2020-01-01 00:00:00+00:00');
INSERT INTO books VALUES(4,'DUNE','Dune','2020-01-01 00:00:00+00:00','0101-01-01 00:00:00+00:00',1.0,'Herbert, Frank','', '', 'Frank Herbert/Dune (4)',0,'u4',0,'2020-01-01 00:00:00+00:00');
INSERT INTO books VALUES(5,'Anonymous Tome','Anonymous Tome',NULL,NULL,1.0,NULL,'', '', 'Unknown/Anonymous Tome (5)',0,'u5',1,'garbage');
INSERT INTO books VALUES(6,'Two Formats','Two Formats','2020-01-01 00:00:00+00:00','2001-02-03 00:00:00+00:00',3.0,'Smith| Jo','', '', 'Jo Smith/Two Formats (6)',0,'u6',0,'2020-01-01 00:00:00+00:00');
INSERT INTO books VALUES(7,'No Data At All','No Data At All','2020-01-01 00:00:00+00:00','0101-01-01 00:00:00+00:00',1.0,'Ghost','', '', 'Ghost/No Data (7)',0,'u7',0,'2020-01-01 00:00:00+00:00');

INSERT INTO authors VALUES(1,'Frank Herbert','Herbert, Frank','');
INSERT INTO authors VALUES(2,'J. R. R. Tolkien','Tolkien, J. R. R.','');
INSERT INTO authors VALUES(3,'Nobody','Nobody','');
INSERT INTO authors VALUES(4,'Smith| Jo','Smith, Jo','');
INSERT INTO authors VALUES(5,'Second Author','Author, Second','');
INSERT INTO books_authors_link VALUES(1,1,1);
INSERT INTO books_authors_link VALUES(2,2,2);
INSERT INTO books_authors_link VALUES(3,3,3);
INSERT INTO books_authors_link VALUES(4,4,1);
INSERT INTO books_authors_link VALUES(5,6,5);
INSERT INTO books_authors_link VALUES(6,6,4);

INSERT INTO tags VALUES(1,'Science Fiction','');
INSERT INTO tags VALUES(2,'Classic','');
INSERT INTO books_tags_link VALUES(1,1,1);
INSERT INTO books_tags_link VALUES(2,1,2);
INSERT INTO books_tags_link VALUES(3,2,2);

INSERT INTO series VALUES(1,'Dune Chronicles','Dune Chronicles','');
INSERT INTO books_series_link VALUES(1,1,1);
INSERT INTO publishers VALUES(1,'Chilton Books','Chilton Books','');
INSERT INTO books_publishers_link VALUES(1,1,1);
INSERT INTO comments VALUES(1,1,'<div>A desert planet.</div>');
INSERT INTO identifiers VALUES(1,1,'isbn','9780441013593');
INSERT INTO identifiers VALUES(2,1,'goodreads','234225');
INSERT INTO ratings VALUES(1,8,'');
INSERT INTO ratings VALUES(2,0,'');
INSERT INTO books_ratings_link VALUES(1,1,1);
INSERT INTO books_ratings_link VALUES(2,2,2);
INSERT INTO languages VALUES(1,'eng','');
INSERT INTO languages VALUES(2,'deu','');
INSERT INTO books_languages_link VALUES(1,1,2,1);
INSERT INTO books_languages_link VALUES(2,1,1,0);

INSERT INTO data VALUES(1,1,'MOBI',111,'Dune');
INSERT INTO data VALUES(2,1,'EPUB',2048,'Dune - Frank Herbert');
INSERT INTO data VALUES(3,2,'EPUB',512,'The Hobbit - J. R. R. Tolkien');
INSERT INTO data VALUES(4,3,'PDF',99,'Only A PDF - Nobody');
INSERT INTO data VALUES(5,4,'epub',700,'DUNE - Frank Herbert');
INSERT INTO data VALUES(6,5,'EPUB',64,'Anonymous Tome - Unknown');
INSERT INTO data VALUES(7,6,'AZW3',5,'x');
INSERT INTO data VALUES(8,6,'EPUB',300,'Two Formats - Jo Smith');
INSERT INTO data VALUES(9,6,'EPUB',301,'second epub row');
";

fn make_library(dir: &std::path::Path, fixture: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let c = Connection::open(dir.join("metadata.db")).unwrap();
    c.execute_batch(CALIBRE_SCHEMA).unwrap();
    c.execute_batch(fixture).unwrap();
}

#[test]
fn imports_epub_books_with_all_metadata() {
    let t = Tmp::new();
    make_library(t.path(), FIXTURE);
    let s = SqliteStore::open_in_memory().unwrap();
    let r = s.import_calibre(t.path()).unwrap();
    assert_eq!(
        r,
        ImportReport {
            imported: 4,        // Dune, The Hobbit, Anonymous Tome, Two Formats
            skipped_no_epub: 2, // Only A PDF, No Data At All
            duplicates: 1,      // DUNE (id 4)
        }
    );
    assert_eq!(s.len(), 4);

    let dune = s
        .all_books()
        .unwrap()
        .into_iter()
        .find(|b| b.meta.title == "Dune")
        .unwrap();
    assert_eq!(
        dune.file,
        "Frank Herbert/Dune (1)/Dune - Frank Herbert.epub"
    );
    assert_eq!(
        dune.cover.as_deref(),
        Some("Frank Herbert/Dune (1)/cover.jpg")
    );
    assert_eq!(dune.size, 2048);
    assert_eq!(dune.meta.authors, vec!["Frank Herbert"]);
    assert_eq!(dune.meta.author_sort, "Herbert, Frank");
    assert_eq!(dune.meta.title_sort, "Dune");
    let mut tags = dune.meta.tags.clone();
    tags.sort();
    assert_eq!(tags, vec!["Classic", "Science Fiction"]);
    assert_eq!(dune.meta.series, "Dune Chronicles");
    assert_eq!(dune.meta.series_index, 1.0);
    assert_eq!(dune.meta.publisher, "Chilton Books");
    assert_eq!(dune.meta.description, "<div>A desert planet.</div>");
    assert_eq!(
        dune.meta.language, "eng",
        "item_order decides the first language"
    );
    assert_eq!(dune.meta.rating, 8);
    assert_eq!(dune.meta.published, "1965-08-01");
    assert_eq!(dune.meta.identifiers.len(), 2);
    assert_eq!(dune.meta.identifiers[0].scheme, "isbn");
    assert_eq!(dune.meta.identifiers[0].value, "9780441013593");
    assert_eq!(dune.meta.identifiers[1].scheme, "goodreads");
    assert_eq!(dune.added.to_rfc3339(), "2021-03-04T05:06:07.123456+00:00");
    assert_eq!(dune.modified.to_rfc3339(), "2022-01-01T10:00:00+00:00");
    assert!(dune.last_read.is_none() && dune.position.is_none());
    assert_eq!(dune.progress, 0.0);
}

#[test]
fn defaults_for_missing_fields() {
    let t = Tmp::new();
    make_library(t.path(), FIXTURE);
    let s = SqliteStore::open_in_memory().unwrap();
    s.import_calibre(t.path()).unwrap();
    let books = s.all_books().unwrap();

    let hobbit = books.iter().find(|b| b.meta.title == "The Hobbit").unwrap();
    assert!(hobbit.cover.is_none(), "has_cover = 0");
    assert_eq!(hobbit.meta.title_sort, "Hobbit, The");
    assert_eq!(hobbit.meta.rating, 0);
    assert_eq!(hobbit.meta.published, "", "year 0101 means unset");
    assert_eq!(hobbit.meta.series, "");
    assert_eq!(hobbit.meta.series_index, 0.0);
    assert_eq!(hobbit.meta.language, "");
    assert_eq!(
        hobbit.file,
        "J. R. R. Tolkien/The Hobbit (2)/The Hobbit - J. R. R. Tolkien.epub"
    );

    let anon = books
        .iter()
        .find(|b| b.meta.title == "Anonymous Tome")
        .unwrap();
    assert_eq!(anon.meta.authors, vec!["Unknown"]);
    assert_eq!(anon.meta.author_sort, "Unknown");
    assert_eq!(
        anon.cover.as_deref(),
        Some("Unknown/Anonymous Tome (5)/cover.jpg")
    );
    // unparsable / NULL timestamps do not fail the import
    assert!(anon.added.timestamp() > 1_600_000_000);

    let two = books
        .iter()
        .find(|b| b.meta.title == "Two Formats")
        .unwrap();
    assert_eq!(
        two.meta.authors,
        vec!["Second Author", "Smith, Jo"],
        "link order, pipes decoded"
    );
    assert_eq!(
        two.file, "Jo Smith/Two Formats (6)/Two Formats - Jo Smith.epub",
        "first epub row wins"
    );
    assert_eq!(two.size, 300);
}

#[test]
fn imported_books_are_searchable_with_fresh_ids() {
    let t = Tmp::new();
    make_library(t.path(), FIXTURE);
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(book(10, "Existing", "Me")).unwrap();
    s.import_calibre(t.path()).unwrap();
    assert_eq!(s.len(), 5);
    let ids = s.ids();
    assert_eq!(ids[0], 10);
    assert!(
        ids[1..].iter().all(|i| *i > 10),
        "new ids continue after existing: {ids:?}"
    );
    assert_eq!(s.search_ids("herbert").len(), 1);
    assert_eq!(s.search_ids("tag:=classic").len(), 2);
    assert_eq!(s.search_ids("series:chronicles").len(), 1);
    assert_eq!(s.next_id(), ids.last().unwrap() + 1);
}

#[test]
fn importing_twice_only_reports_duplicates() {
    let t = Tmp::new();
    make_library(t.path(), FIXTURE);
    let s = SqliteStore::open_in_memory().unwrap();
    s.import_calibre(t.path()).unwrap();
    let r = s.import_calibre(t.path()).unwrap();
    assert_eq!(
        r,
        ImportReport {
            imported: 0,
            skipped_no_epub: 2,
            duplicates: 5, // the 4 imported plus the DUNE twin
        }
    );
    assert_eq!(s.len(), 4);
}

#[test]
fn existing_book_with_same_title_and_author_counts_as_duplicate() {
    let t = Tmp::new();
    make_library(t.path(), FIXTURE);
    let s = SqliteStore::open_in_memory().unwrap();
    s.put(book(1, "the hobbit", "j. r. r. tolkien")).unwrap();
    let r = s.import_calibre(t.path()).unwrap();
    assert_eq!(r.imported, 3);
    assert_eq!(r.duplicates, 2);
}

#[test]
fn empty_calibre_library() {
    let t = Tmp::new();
    make_library(t.path(), "");
    let s = SqliteStore::open_in_memory().unwrap();
    assert_eq!(s.import_calibre(t.path()).unwrap(), ImportReport::default());
    assert!(s.is_empty());
}

#[test]
fn calibre_import_does_not_modify_the_source_or_copy_files() {
    let t = Tmp::new();
    make_library(t.path(), FIXTURE);
    let before = std::fs::read(t.join("metadata.db")).unwrap();
    let s = SqliteStore::open_in_memory().unwrap();
    s.import_calibre(t.path()).unwrap();
    assert_eq!(std::fs::read(t.join("metadata.db")).unwrap(), before);
    let entries: Vec<_> = std::fs::read_dir(t.path()).unwrap().flatten().collect();
    assert!(
        entries.len() <= 2,
        "only metadata.db (and maybe its journal) expected"
    );
}

#[test]
fn calibre_import_errors() {
    let t = Tmp::new();
    let s = SqliteStore::open_in_memory().unwrap();
    let err = s.import_calibre(&t.join("nothing")).unwrap_err();
    assert!(err.to_string().contains("no metadata.db"), "{err}");

    // a sqlite file that is not Calibre
    std::fs::create_dir_all(t.join("other")).unwrap();
    Connection::open(t.join("other/metadata.db"))
        .unwrap()
        .execute_batch("CREATE TABLE x(y);")
        .unwrap();
    let err = s.import_calibre(&t.join("other")).unwrap_err();
    assert!(
        format!("{err:#}").contains("not a Calibre metadata.db"),
        "{err:#}"
    );

    // garbage file
    std::fs::create_dir_all(t.join("junk")).unwrap();
    std::fs::write(t.join("junk/metadata.db"), vec![1u8; 4000]).unwrap();
    assert!(s.import_calibre(&t.join("junk")).is_err());
    assert!(s.is_empty());
}

#[test]
fn ratings_above_ten_are_clamped() {
    let t = Tmp::new();
    make_library(
        t.path(),
        "INSERT INTO books VALUES(1,'T','T','2020-01-01 00:00:00+00:00','2020-01-01 00:00:00+00:00',1.0,'A','','','p',0,'u',0,'2020-01-01 00:00:00+00:00');
         INSERT INTO ratings VALUES(1,40,''); INSERT INTO books_ratings_link VALUES(1,1,1);
         INSERT INTO data VALUES(1,1,'EPUB',1,'n');",
    );
    let s = SqliteStore::open_in_memory().unwrap();
    s.import_calibre(t.path()).unwrap();
    assert_eq!(s.get(1).unwrap().meta.rating, 10);
}
