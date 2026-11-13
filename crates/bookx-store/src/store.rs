//! The SQLite store: schema, migrations, CRUD, search, collections, status, backup.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bookx_core::contracts::Store;
use bookx_core::library::Book;
use bookx_core::query::Query;
use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;

/// Schema version written to `PRAGMA user_version` by the newest migration.
pub const SCHEMA_VERSION: i32 = 6;

/// Catalogue persisted in SQLite. Cheap to share between threads (`Send + Sync`).
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

/// Sort orders backed by indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Title,
    Author,
    Series,
    Added,
    LastRead,
}

/// Reading state of a book, independent of the progress percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingStatus {
    New,
    Reading,
    OnHold,
    Finished,
}

impl ReadingStatus {
    pub const ALL: [ReadingStatus; 4] = [Self::New, Self::Reading, Self::OnHold, Self::Finished];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Reading => "reading",
            Self::OnHold => "on_hold",
            Self::Finished => "finished",
        }
    }

    pub fn parse(s: &str) -> Option<ReadingStatus> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// A named group of books.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub books: usize,
}

/// One content-text search result. `snippet` wraps matches in `<mark>…</mark>` and is NOT escaped.
#[derive(Debug, Clone, PartialEq)]
pub struct TextHit {
    pub id: u64,
    pub snippet: String,
}

/// Result of [`SqliteStore::import_json_catalog`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JsonImportReport {
    pub imported: usize,
    /// Books whose id was already in the store (or repeated in the file).
    pub skipped: usize,
}

#[derive(Deserialize, Default)]
struct JsonCatalog {
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    books: Vec<Book>,
}

// -- migrations -----------------------------------------------------------

fn migrate(conn: &mut Connection) -> Result<()> {
    let cur: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if cur > SCHEMA_VERSION {
        bail!("database schema version {cur} is newer than supported version {SCHEMA_VERSION}");
    }
    for v in cur + 1..=SCHEMA_VERSION {
        let tx = conn.transaction()?;
        match v {
            1 => migrate_v1(&tx)?,
            2 => migrate_v2(&tx)?,
            3 => migrate_v3(&tx)?,
            4 => migrate_v4(&tx)?,
            5 => migrate_v5(&tx)?,
            6 => migrate_v6(&tx)?,
            _ => bail!("no migration to schema version {v}"),
        }
        tx.execute_batch(&format!("PRAGMA user_version = {v}"))?;
        tx.commit()?;
    }
    Ok(())
}

fn migrate_v1(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
         INSERT INTO meta(key, value) VALUES('next_id', 1);
         CREATE TABLE books(
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             title_sort TEXT NOT NULL,
             author_sort TEXT NOT NULL,
             series TEXT NOT NULL,
             series_index REAL NOT NULL,
             added INTEGER NOT NULL,
             modified INTEGER NOT NULL,
             last_read INTEGER,
             progress REAL NOT NULL,
             data TEXT NOT NULL
         );
         CREATE INDEX books_title_sort ON books(title_sort COLLATE NOCASE);
         CREATE INDEX books_author_sort ON books(author_sort COLLATE NOCASE);
         CREATE INDEX books_series ON books(series COLLATE NOCASE, series_index);
         CREATE INDEX books_added ON books(added);
         CREATE INDEX books_last_read ON books(last_read);",
    )?;
    Ok(())
}

fn migrate_v2(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE VIRTUAL TABLE fts_meta USING fts5(
             title, authors, series, tags, publisher, description, tokenize='trigram');
         CREATE VIRTUAL TABLE fts_text USING fts5(
             text, tokenize='porter unicode61 remove_diacritics 2');",
    )?;
    let rows: Vec<(i64, String)> = {
        let mut st = tx.prepare("SELECT id, data FROM books")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (id, data) in rows {
        if let Ok(book) = serde_json::from_str::<Book>(&data) {
            index_meta(tx, id, &book)?;
        }
    }
    Ok(())
}

fn migrate_v3(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE collections(
             id INTEGER PRIMARY KEY,
             name TEXT NOT NULL UNIQUE COLLATE NOCASE
         );
         CREATE TABLE collection_books(
             collection INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
             book INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
             added INTEGER NOT NULL,
             PRIMARY KEY(collection, book)
         ) WITHOUT ROWID;
         CREATE INDEX collection_books_book ON collection_books(book);
         CREATE TABLE book_status(
             book INTEGER PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
             status TEXT NOT NULL CHECK(status IN ('new','reading','on_hold','finished'))
         );",
    )?;
    Ok(())
}

fn migrate_v4(tx: &Connection) -> Result<()> {
    // which version of each book's content text is indexed (the app re-indexes changed books)
    tx.execute_batch(
        "CREATE TABLE text_stamps(
             book INTEGER PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
             stamp INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

fn migrate_v5(tx: &Connection) -> Result<()> {
    // where each field of a book came from (a file, a metadata source, the user)
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS provenance(
             book INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
             field TEXT NOT NULL,
             source TEXT NOT NULL,
             at INTEGER NOT NULL,
             PRIMARY KEY(book, field)
         ) WITHOUT ROWID;",
    )?;
    Ok(())
}

fn migrate_v6(tx: &Connection) -> Result<()> {
    // one book can hold several format files; author aliases (pen names, name variants)
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS book_formats(
             book INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
             file TEXT NOT NULL,
             format TEXT NOT NULL,
             size INTEGER NOT NULL DEFAULT 0,
             main INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY(book, file)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS book_formats_format ON book_formats(format);
         CREATE TABLE IF NOT EXISTS author_aliases(
             alias TEXT NOT NULL COLLATE NOCASE PRIMARY KEY,
             canonical TEXT NOT NULL
         ) WITHOUT ROWID;",
    )?;
    // existing books: their one file is the main format
    let books = all_books_conn(tx)?;
    for b in &books {
        index_formats(tx, b)?;
    }
    Ok(())
}

// -- row helpers ----------------------------------------------------------

fn to_i64(id: u64) -> Result<i64> {
    i64::try_from(id).map_err(|_| anyhow::anyhow!("book id {id} is too large for the store"))
}

fn index_meta(conn: &Connection, id: i64, b: &Book) -> Result<()> {
    let m = &b.meta;
    conn.prepare_cached("DELETE FROM fts_meta WHERE rowid = ?1")?
        .execute([id])?;
    conn.prepare_cached(
        "INSERT INTO fts_meta(rowid, title, authors, series, tags, publisher, description)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?
    .execute(params![
        id,
        m.title,
        m.authors.join("\n"),
        m.series,
        m.tags.join("\n"),
        m.publisher,
        m.description
    ])?;
    Ok(())
}

/// Rewrites the `book_formats` rows of a book: the main file and the further formats.
fn index_formats(conn: &Connection, b: &Book) -> Result<()> {
    let id = to_i64(b.id)?;
    conn.prepare_cached("DELETE FROM book_formats WHERE book = ?1")?
        .execute([id])?;
    let mut ins = conn.prepare_cached(
        "INSERT OR REPLACE INTO book_formats(book, file, format, size, main) VALUES(?1, ?2, ?3, ?4, ?5)",
    )?;
    for (i, f) in b.all_formats().iter().enumerate() {
        ins.execute(params![
            id,
            f.file,
            f.format,
            f.size as i64,
            (i == 0) as i64
        ])?;
    }
    Ok(())
}

pub(crate) fn put_row(conn: &Connection, b: &Book) -> Result<()> {
    let id = to_i64(b.id)?;
    let data = serde_json::to_string(b)?;
    conn.prepare_cached(
        "INSERT INTO books(id, title, title_sort, author_sort, series, series_index,
                           added, modified, last_read, progress, data)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET
             title = excluded.title, title_sort = excluded.title_sort,
             author_sort = excluded.author_sort, series = excluded.series,
             series_index = excluded.series_index, added = excluded.added,
             modified = excluded.modified, last_read = excluded.last_read,
             progress = excluded.progress, data = excluded.data",
    )?
    .execute(params![
        id,
        b.meta.title,
        b.meta.title_sort,
        b.meta.author_sort,
        b.meta.series,
        b.meta.series_index,
        b.added.timestamp_millis(),
        b.modified.timestamp_millis(),
        b.last_read.map(|d| d.timestamp_millis()),
        b.progress,
        data
    ])?;
    index_meta(conn, id, b)?;
    index_formats(conn, b)?;
    conn.prepare_cached("UPDATE meta SET value = max(value, ?1) WHERE key = 'next_id'")?
        .execute([id.saturating_add(1)])?;
    Ok(())
}

fn load(conn: &Connection, id: u64) -> Option<Book> {
    let id = i64::try_from(id).ok()?;
    let data: String = conn
        .prepare_cached("SELECT data FROM books WHERE id = ?1")
        .ok()?
        .query_row([id], |r| r.get(0))
        .optional()
        .ok()??;
    serde_json::from_str(&data).ok()
}

fn exists(conn: &Connection, id: u64) -> Result<bool> {
    let Ok(id) = i64::try_from(id) else {
        return Ok(false);
    };
    Ok(conn
        .prepare_cached("SELECT 1 FROM books WHERE id = ?1")?
        .exists([id])?)
}

pub(crate) fn alloc_id(conn: &Connection) -> Result<u64> {
    let next: i64 = conn.query_row("SELECT value FROM meta WHERE key = 'next_id'", [], |r| {
        r.get(0)
    })?;
    conn.execute(
        "UPDATE meta SET value = ?1 WHERE key = 'next_id'",
        [next + 1],
    )?;
    Ok(next as u64)
}

pub(crate) fn all_books_conn(conn: &Connection) -> Result<Vec<Book>> {
    let mut st = conn.prepare("SELECT data FROM books ORDER BY id")?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        if let Ok(b) = serde_json::from_str::<Book>(&r?) {
            out.push(b);
        }
    }
    Ok(out)
}

/// True when [`SqliteStore::search_ids`] answers `q` through the FTS index: one ASCII word of at
/// least three letters or digits.
pub fn is_fast_query(q: &str) -> bool {
    let t = q.trim();
    t.chars().count() >= 3 && t.chars().all(|c| c.is_ascii_alphanumeric())
}

fn fts_phrase_query(q: &str) -> String {
    q.split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .map(|w| format!("\"{w}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

fn integrity_messages(conn: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    let run = || -> rusqlite::Result<Vec<String>> {
        let mut st = conn.prepare("PRAGMA integrity_check")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    };
    match run() {
        Ok(rows) => out.extend(rows.into_iter().filter(|s| s != "ok")),
        Err(e) => out.push(e.to_string()),
    }
    if out.is_empty() {
        let counts = || -> rusqlite::Result<(i64, i64)> {
            Ok((
                conn.query_row("SELECT count(*) FROM books", [], |r| r.get(0))?,
                conn.query_row("SELECT count(*) FROM fts_meta", [], |r| r.get(0))?,
            ))
        };
        match counts() {
            Ok((b, f)) if b != f => out.push(format!(
                "search index out of sync: {b} books but {f} indexed rows"
            )),
            Ok(_) => {}
            Err(e) => out.push(e.to_string()),
        }
    }
    out
}

fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    path.with_file_name(name)
}

// -- the store ------------------------------------------------------------

impl SqliteStore {
    /// Opens (creating if needed) the database file and migrates it to the latest schema.
    pub fn open(path: &Path) -> Result<SqliteStore> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let conn = Connection::open(path)
            .with_context(|| format!("cannot open database {}", path.display()))?;
        Self::init(conn).with_context(|| format!("cannot use database {}", path.display()))
    }

    pub fn open_in_memory() -> Result<SqliteStore> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<SqliteStore> {
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
        migrate(&mut conn)?;
        Ok(SqliteStore {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `PRAGMA user_version` of the open database.
    pub fn schema_version(&self) -> i32 {
        self.lock()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.lock()
            .query_row("SELECT count(*) FROM books", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, id: u64) -> bool {
        exists(&self.lock(), id).unwrap_or(false)
    }

    /// The id the next new book should get (always above every stored id).
    pub fn next_id(&self) -> u64 {
        self.lock()
            .query_row("SELECT value FROM meta WHERE key = 'next_id'", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(1) as u64
    }

    /// The format files of a book: `(file, format, size, is the main file)`.
    pub fn formats_of(&self, book: u64) -> Vec<(String, String, u64, bool)> {
        let conn = self.lock();
        let Ok(mut st) = conn.prepare(
            "SELECT file, format, size, main FROM book_formats WHERE book = ?1 ORDER BY main DESC, file",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([book as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? != 0,
            ))
        }) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    /// Books that have a file in `format` (main or further).
    pub fn books_with_format(&self, format: &str) -> Vec<u64> {
        let conn = self.lock();
        let Ok(mut st) = conn.prepare(
            "SELECT DISTINCT book FROM book_formats WHERE format = ?1 COLLATE NOCASE ORDER BY book",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([format], |r| r.get::<_, i64>(0)) else {
            return Vec::new();
        };
        rows.flatten().map(|i| i as u64).collect()
    }

    /// All author aliases as `(alias, canonical name)`, sorted.
    pub fn author_aliases(&self) -> Vec<(String, String)> {
        let conn = self.lock();
        let Ok(mut st) =
            conn.prepare("SELECT alias, canonical FROM author_aliases ORDER BY canonical, alias")
        else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    /// Declares `alias` to be the same author as `canonical`. An alias that was a canonical name
    /// of others is re-pointed, so chains never form.
    pub fn set_author_alias(&self, alias: &str, canonical: &str) -> Result<()> {
        let (alias, canonical) = (alias.trim(), canonical.trim());
        if alias.is_empty() || canonical.is_empty() {
            bail!("an alias needs a name on both sides");
        }
        if alias.eq_ignore_ascii_case(canonical) {
            bail!("a name cannot be an alias of itself");
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        // the canonical name may itself be an alias: follow it
        let target: String = tx
            .query_row(
                "SELECT canonical FROM author_aliases WHERE alias = ?1",
                [canonical],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| canonical.to_string());
        if alias.eq_ignore_ascii_case(&target) {
            bail!("that would make a name an alias of itself");
        }
        tx.execute(
            "UPDATE author_aliases SET canonical = ?1 WHERE canonical = ?2 COLLATE NOCASE",
            params![target, alias],
        )?;
        tx.execute(
            "INSERT INTO author_aliases(alias, canonical) VALUES(?1, ?2)
             ON CONFLICT(alias) DO UPDATE SET canonical = excluded.canonical",
            params![alias, target],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_author_alias(&self, alias: &str) -> Result<bool> {
        Ok(self.lock().execute(
            "DELETE FROM author_aliases WHERE alias = ?1",
            [alias.trim()],
        )? > 0)
    }

    /// Reserves and returns a fresh id.
    pub fn allocate_id(&self) -> Result<u64> {
        alloc_id(&self.lock())
    }

    /// Stores many books in one transaction (all or nothing).
    pub fn put_many(&self, books: &[Book]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for b in books {
            put_row(&tx, b)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Atomically applies a catalogue change: stores `upserts`, deletes `removed` (with their
    /// search rows) and records the id counter. All or nothing.
    pub fn commit_catalog(&self, next_id: u64, upserts: &[&Book], removed: &[u64]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for id in removed {
            let Ok(id) = i64::try_from(*id) else {
                continue;
            };
            tx.execute("DELETE FROM books WHERE id = ?1", [id])?;
            tx.execute("DELETE FROM fts_meta WHERE rowid = ?1", [id])?;
            tx.execute("DELETE FROM fts_text WHERE rowid = ?1", [id])?;
        }
        for b in upserts {
            put_row(&tx, b)?;
        }
        tx.execute(
            "UPDATE meta SET value = max(value, ?1) WHERE key = 'next_id'",
            [to_i64(next_id)?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Changes whenever *another* connection to the same database file committed something since
    /// this one last looked (SQLite's `data_version`). A server polling the library uses it to know
    /// when to reload.
    pub fn data_version(&self) -> i64 {
        self.lock()
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// The whole catalogue as the JSON export format: `{"next_id": …, "books": […]}`.
    pub fn export_catalog_json(&self) -> Result<String> {
        #[derive(serde::Serialize)]
        struct Out {
            next_id: u64,
            books: Vec<Book>,
        }
        let out = Out {
            next_id: self.next_id(),
            books: self.all_books()?,
        };
        Ok(serde_json::to_string_pretty(&out)?)
    }

    /// Every book, ordered by id. Rows whose JSON cannot be read are skipped.
    pub fn all_books(&self) -> Result<Vec<Book>> {
        all_books_conn(&self.lock())
    }

    /// Ids ordered through an index. Ties break on id.
    pub fn ids_sorted(&self, order: Order, ascending: bool) -> Vec<u64> {
        let col = match order {
            Order::Title => "title_sort COLLATE NOCASE",
            Order::Author => "author_sort COLLATE NOCASE",
            Order::Series => "series COLLATE NOCASE, series_index",
            Order::Added => "added",
            Order::LastRead => "last_read",
        };
        let dir = if ascending { "ASC" } else { "DESC" };
        let sql = format!("SELECT id FROM books ORDER BY {col} {dir}, id ASC");
        let conn = self.lock();
        let Ok(mut st) = conn.prepare(&sql) else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([], |r| r.get::<_, i64>(0)) else {
            return Vec::new();
        };
        rows.flatten().map(|i| i as u64).collect()
    }

    // -- search -----------------------------------------------------------

    /// Ids matching a Calibre-style expression, ascending. Same results as filtering every book with
    /// [`Query`]; single plain words use the FTS index to avoid loading the whole catalogue.
    pub fn search_ids(&self, q: &str) -> Vec<u64> {
        let query = Query::parse(q);
        let conn = self.lock();
        if is_fast_query(q) {
            let candidates: Vec<i64> = conn
                .prepare_cached("SELECT rowid FROM fts_meta WHERE fts_meta MATCH ?1 ORDER BY rowid")
                .and_then(|mut st| {
                    st.query_map([format!("\"{}\"", q.trim())], |r| r.get(0))?
                        .collect()
                })
                .unwrap_or_default();
            return candidates
                .into_iter()
                .filter_map(|id| load(&conn, id as u64))
                .filter(|b| query.matches(b))
                .map(|b| b.id)
                .collect();
        }
        all_books_conn(&conn)
            .unwrap_or_default()
            .into_iter()
            .filter(|b| query.matches(b))
            .map(|b| b.id)
            .collect()
    }

    /// Replaces the indexed content text of a book.
    pub fn index_book_text(&self, id: u64, text: &str) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        index_text(&tx, id, text)?;
        tx.commit()?;
        Ok(())
    }

    /// Like [`SqliteStore::index_book_text`], also recording which version (`stamp`) was indexed.
    pub fn index_book_text_stamped(&self, id: u64, stamp: i64, text: &str) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        index_text(&tx, id, text)?;
        tx.execute(
            "INSERT INTO text_stamps(book, stamp) VALUES(?1, ?2)
             ON CONFLICT(book) DO UPDATE SET stamp = excluded.stamp",
            params![id as i64, stamp],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The stamp recorded for each book with indexed text.
    pub fn text_stamps(&self) -> std::collections::HashMap<u64, i64> {
        let conn = self.lock();
        let Ok(mut st) = conn.prepare("SELECT book, stamp FROM text_stamps") else {
            return Default::default();
        };
        let Ok(rows) = st.query_map([], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)?))
        }) else {
            return Default::default();
        };
        rows.flatten().collect()
    }

    /// Indexes the text of many books in one transaction.
    pub fn index_book_texts(&self, texts: &[(u64, String)]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for (id, text) in texts {
            index_text(&tx, *id, text)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Searches indexed content text; best match first. Every word must occur (stemmed matching).
    pub fn search_text(&self, query: &str, limit: usize) -> Result<Vec<TextHit>> {
        let fts = fts_phrase_query(query);
        if fts.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let mut st = conn.prepare_cached(
            "SELECT rowid, snippet(fts_text, 0, '<mark>', '</mark>', '…', 16)
             FROM fts_text WHERE fts_text MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let rows = st.query_map(params![fts, limit as i64], |r| {
            Ok(TextHit {
                id: r.get::<_, i64>(0)? as u64,
                snippet: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Number of books that have content text indexed.
    pub fn indexed_text_count(&self) -> usize {
        self.lock()
            .query_row("SELECT count(*) FROM fts_text", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    // -- JSON catalogue migration ----------------------------------------

    /// Imports the old `bookx-library.json`. Books whose id is already present are left untouched, so
    /// running it again adds nothing. Ids and `next_id` are preserved.
    pub fn import_json_catalog(&self, path: &Path) -> Result<JsonImportReport> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read catalogue {}", path.display()))?;
        let cat: JsonCatalog = serde_json::from_str(&text).context("corrupt library catalogue")?;
        let mut report = JsonImportReport::default();
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for b in &cat.books {
            if exists(&tx, b.id)? {
                report.skipped += 1;
            } else {
                put_row(&tx, b)?;
                report.imported += 1;
            }
        }
        tx.execute(
            "UPDATE meta SET value = max(value, ?1) WHERE key = 'next_id'",
            [to_i64(cat.next_id)?],
        )?;
        tx.commit()?;
        Ok(report)
    }

    // -- collections ------------------------------------------------------

    pub fn create_collection(&self, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            bail!("collection name is empty");
        }
        let conn = self.lock();
        match conn.execute("INSERT INTO collections(name) VALUES(?1)", [name]) {
            Ok(_) => Ok(conn.last_insert_rowid()),
            Err(e) if is_constraint(&e) => bail!("collection `{name}` already exists"),
            Err(e) => Err(e.into()),
        }
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("collection name is empty");
        }
        let conn = self.lock();
        match conn.execute(
            "UPDATE collections SET name = ?1 WHERE id = ?2",
            params![name, id],
        ) {
            Ok(0) => bail!("no such collection {id}"),
            Ok(_) => Ok(()),
            Err(e) if is_constraint(&e) => bail!("collection `{name}` already exists"),
            Err(e) => Err(e.into()),
        }
    }

    /// Deletes the collection (not its books). False when it did not exist.
    pub fn delete_collection(&self, id: i64) -> Result<bool> {
        Ok(self
            .lock()
            .execute("DELETE FROM collections WHERE id = ?1", [id])?
            > 0)
    }

    /// All collections ordered by name, with the number of books in each.
    pub fn list_collections(&self) -> Result<Vec<Collection>> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT c.id, c.name, count(cb.book) FROM collections c
             LEFT JOIN collection_books cb ON cb.collection = c.id
             GROUP BY c.id ORDER BY c.name COLLATE NOCASE",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Collection {
                id: r.get(0)?,
                name: r.get(1)?,
                books: r.get::<_, i64>(2)? as usize,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds a book to a collection (no-op when already there).
    pub fn add_to_collection(&self, collection: i64, book: u64) -> Result<()> {
        let conn = self.lock();
        if !conn
            .prepare_cached("SELECT 1 FROM collections WHERE id = ?1")?
            .exists([collection])?
        {
            bail!("no such collection {collection}");
        }
        if !exists(&conn, book)? {
            bail!("no such book {book}");
        }
        conn.execute(
            "INSERT OR IGNORE INTO collection_books(collection, book, added) VALUES(?1, ?2, ?3)",
            params![collection, book as i64, Utc::now().timestamp_millis()],
        )?;
        Ok(())
    }

    /// True when the book was in the collection.
    pub fn remove_from_collection(&self, collection: i64, book: u64) -> Result<bool> {
        let Ok(book) = i64::try_from(book) else {
            return Ok(false);
        };
        Ok(self.lock().execute(
            "DELETE FROM collection_books WHERE collection = ?1 AND book = ?2",
            params![collection, book],
        )? > 0)
    }

    /// Book ids of a collection, ascending. Empty for an unknown collection.
    pub fn collection_books(&self, collection: i64) -> Vec<u64> {
        let conn = self.lock();
        let Ok(mut st) =
            conn.prepare("SELECT book FROM collection_books WHERE collection = ?1 ORDER BY book")
        else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([collection], |r| r.get::<_, i64>(0)) else {
            return Vec::new();
        };
        rows.flatten().map(|b| b as u64).collect()
    }

    /// Collection ids the book belongs to, ascending.
    pub fn collections_of(&self, book: u64) -> Vec<i64> {
        let Ok(book) = i64::try_from(book) else {
            return Vec::new();
        };
        let conn = self.lock();
        let Ok(mut st) = conn
            .prepare("SELECT collection FROM collection_books WHERE book = ?1 ORDER BY collection")
        else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([book], |r| r.get::<_, i64>(0)) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    // -- reading status ---------------------------------------------------

    pub fn set_status(&self, book: u64, status: ReadingStatus) -> Result<()> {
        let conn = self.lock();
        if !exists(&conn, book)? {
            bail!("no such book {book}");
        }
        conn.execute(
            "INSERT INTO book_status(book, status) VALUES(?1, ?2)
             ON CONFLICT(book) DO UPDATE SET status = excluded.status",
            params![book as i64, status.as_str()],
        )?;
        Ok(())
    }

    /// `None` when the book does not exist; books never set are [`ReadingStatus::New`].
    pub fn get_status(&self, book: u64) -> Option<ReadingStatus> {
        let conn = self.lock();
        if !exists(&conn, book).ok()? {
            return None;
        }
        let s: Option<String> = conn
            .query_row(
                "SELECT status FROM book_status WHERE book = ?1",
                [book as i64],
                |r| r.get(0),
            )
            .optional()
            .ok()?;
        Some(
            s.and_then(|s| ReadingStatus::parse(&s))
                .unwrap_or(ReadingStatus::New),
        )
    }

    /// Ids with the given status, ascending ([`ReadingStatus::New`] includes never-set books).
    pub fn books_with_status(&self, status: ReadingStatus) -> Vec<u64> {
        let conn = self.lock();
        let sql = if status == ReadingStatus::New {
            "SELECT b.id FROM books b LEFT JOIN book_status s ON s.book = b.id
             WHERE s.status IS NULL OR s.status = ?1 ORDER BY b.id"
        } else {
            "SELECT b.id FROM books b JOIN book_status s ON s.book = b.id
             WHERE s.status = ?1 ORDER BY b.id"
        };
        let Ok(mut st) = conn.prepare(sql) else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([status.as_str()], |r| r.get::<_, i64>(0)) else {
            return Vec::new();
        };
        rows.flatten().map(|b| b as u64).collect()
    }

    // -- backup and integrity --------------------------------------------

    /// Writes a consistent copy of the database to `path` (SQLite online backup), replacing any
    /// existing file there.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let tmp = tmp_sibling(path);
        let result = (|| -> Result<()> {
            let mut dst = Connection::open(&tmp)?;
            {
                let conn = self.lock();
                let backup = rusqlite::backup::Backup::new(&conn, &mut dst)?;
                backup.run_to_completion(256, Duration::from_millis(0), None)?;
            }
            drop(dst);
            std::fs::rename(&tmp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.with_context(|| format!("backup to {} failed", path.display()))
    }

    /// Replaces the whole content of this store with the backup at `path`. The backup is validated
    /// first; on error the store is unchanged. Older backups are migrated to the current schema.
    pub fn restore_from(&self, path: &Path) -> Result<()> {
        let src = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("cannot open backup {}", path.display()))?;
        let version: i32 = src
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .with_context(|| format!("{} is not a Bookx database", path.display()))?;
        if version > SCHEMA_VERSION {
            bail!(
                "backup schema version {version} is newer than supported version {SCHEMA_VERSION}"
            );
        }
        let has_books = src
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'books'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !has_books {
            bail!("{} is not a Bookx database", path.display());
        }
        let problems = integrity_messages_basic(&src);
        if let Some(p) = problems.first() {
            bail!("backup {} is damaged: {p}", path.display());
        }
        let mut conn = self.lock();
        {
            let backup = rusqlite::backup::Backup::new(&src, &mut conn)?;
            backup.run_to_completion(256, Duration::from_millis(0), None)?;
        }
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        migrate(&mut conn)?;
        Ok(())
    }

    /// Problems found in the open database; empty when everything is fine.
    pub fn check_integrity(&self) -> Vec<String> {
        integrity_messages(&self.lock())
    }

    /// Like [`check_integrity`](Self::check_integrity) for a database file that may be unreadable.
    pub fn check_file(path: &Path) -> Vec<String> {
        match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(conn) => integrity_messages(&conn),
            Err(e) => vec![e.to_string()],
        }
    }
}

fn integrity_messages_basic(conn: &Connection) -> Vec<String> {
    let run = || -> rusqlite::Result<Vec<String>> {
        let mut st = conn.prepare("PRAGMA quick_check")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    };
    match run() {
        Ok(rows) => rows.into_iter().filter(|s| s != "ok").collect(),
        Err(e) => vec![e.to_string()],
    }
}

fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

fn index_text(conn: &Connection, id: u64, text: &str) -> Result<()> {
    if !exists(conn, id)? {
        bail!("no such book {id}");
    }
    let id = id as i64;
    conn.prepare_cached("DELETE FROM fts_text WHERE rowid = ?1")?
        .execute([id])?;
    conn.prepare_cached("INSERT INTO fts_text(rowid, text) VALUES(?1, ?2)")?
        .execute(params![id, text])?;
    Ok(())
}

impl Store for SqliteStore {
    fn ids(&self) -> Vec<u64> {
        let conn = self.lock();
        let Ok(mut st) = conn.prepare("SELECT id FROM books ORDER BY id") else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([], |r| r.get::<_, i64>(0)) else {
            return Vec::new();
        };
        rows.flatten().map(|i| i as u64).collect()
    }

    fn get(&self, id: u64) -> Option<Book> {
        load(&self.lock(), id)
    }

    fn put(&self, book: Book) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        put_row(&tx, &book)?;
        tx.commit()?;
        Ok(())
    }

    fn remove(&self, id: u64) -> Result<bool> {
        let Ok(id) = i64::try_from(id) else {
            return Ok(false);
        };
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let n = tx.execute("DELETE FROM books WHERE id = ?1", [id])?;
        tx.execute("DELETE FROM fts_meta WHERE rowid = ?1", [id])?;
        tx.execute("DELETE FROM fts_text WHERE rowid = ?1", [id])?;
        tx.commit()?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_query_quotes_every_word() {
        assert_eq!(fts_phrase_query("fox  lazy"), "\"fox\" \"lazy\"");
        assert_eq!(fts_phrase_query("\"fox\" AND"), "\"fox\" \"AND\"");
        assert_eq!(fts_phrase_query("a\"b"), "\"ab\"");
        assert_eq!(fts_phrase_query(" - ( ) * "), "");
        assert_eq!(fts_phrase_query(""), "");
        assert_eq!(fts_phrase_query("日本語 текст"), "\"日本語\" \"текст\"");
    }

    #[test]
    fn fast_query_rules() {
        assert!(is_fast_query("abc"));
        assert!(!is_fast_query("ab"));
        assert!(!is_fast_query("a b c"));
        assert!(!is_fast_query("ab-c"));
    }

    #[test]
    fn temp_sibling_stays_in_the_same_directory() {
        let a = tmp_sibling(Path::new("/x/y/z.db"));
        let b = tmp_sibling(Path::new("/x/y/z.db"));
        assert_eq!(a.parent(), Some(Path::new("/x/y")));
        assert!(a
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("z.db.tmp-"));
        assert_ne!(a, b);
    }

    #[test]
    fn ids_convert_safely() {
        assert_eq!(to_i64(5).unwrap(), 5);
        assert!(to_i64(u64::MAX).is_err());
    }

    #[test]
    fn constraint_errors_are_recognised() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t(x UNIQUE); INSERT INTO t VALUES(1);")
            .unwrap();
        let e = c.execute("INSERT INTO t VALUES(1)", []).unwrap_err();
        assert!(is_constraint(&e));
        assert!(!is_constraint(&rusqlite::Error::QueryReturnedNoRows));
    }

    #[test]
    fn migration_steps_are_contiguous() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate(&mut c).unwrap();
        let v: i32 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        // running again is a no-op
        migrate(&mut c).unwrap();
    }
}
