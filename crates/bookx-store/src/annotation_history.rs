//! The annotation history in the catalogue database (`annotation_history` table).
//!
//! Every write of a book row diffs the book's old and new highlights and bookmarks (by id) **in the
//! same transaction** and inserts one row per added, edited or deleted record, with who was
//! responsible and the complete record as it was before. No caller can change an annotation without
//! an entry, and a crash cannot leave a change without its entry (or the other way round). Library
//! backups and snapshots carry the table because they copy the database; the JSON and Calibre
//! exports read only the books and never include it. The table has no foreign key to `books`: the
//! entries (with a snapshot of title and authors) outlive a deleted book.
//!
//! Retention ([`SqliteStore::history_prune`]): rows older than 90 days beyond the newest 500, then
//! hard caps on the number of rows and on the bytes of stored text. It runs when the database opens
//! and at most once a day while it stays open.

use std::cell::Cell;

use anyhow::Result;
use bookx_core::annotation_journal::{
    diff, Action, Entry, Filter, Kind, Record, Snap, Source, KEEP_DAYS, KEEP_MIN, MAX_BYTES,
    MAX_ENTRIES,
};
use bookx_core::library::{Book, Bookmark, Highlight};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

use crate::store::SqliteStore;

/// One day, in milliseconds: the retention pass runs at most this often.
const PRUNE_EVERY_MS: i64 = 24 * 3600 * 1000;
/// A record bigger than this is not journalled (no real highlight is).
const MAX_RECORD_BYTES: usize = 256 * 1024;

pub(crate) fn migrate_v10(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS annotation_history(
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             at INTEGER NOT NULL,
             book_id INTEGER NOT NULL,
             title TEXT NOT NULL,
             authors TEXT NOT NULL,
             kind TEXT NOT NULL,
             action TEXT NOT NULL,
             source TEXT NOT NULL,
             group_id INTEGER NOT NULL,
             annotation_id TEXT NOT NULL,
             record TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS annotation_history_book ON annotation_history(book_id, at);
         CREATE INDEX IF NOT EXISTS annotation_history_at ON annotation_history(at);
         CREATE INDEX IF NOT EXISTS annotation_history_source ON annotation_history(source);
         CREATE INDEX IF NOT EXISTS annotation_history_group ON annotation_history(group_id);",
    )?;
    Ok(())
}

#[derive(Deserialize, Default)]
struct Annotations {
    #[serde(default)]
    highlights: Vec<Highlight>,
    #[serde(default)]
    bookmarks: Vec<Bookmark>,
}

/// The group of one write (one transaction): allocated when the first entry is inserted.
#[derive(Default)]
pub(crate) struct GroupId(Cell<Option<i64>>);

impl GroupId {
    fn get(&self, conn: &Connection) -> Result<i64> {
        if let Some(g) = self.0.get() {
            return Ok(g);
        }
        let g: i64 = conn.query_row(
            "SELECT coalesce(max(group_id), 0) + 1 FROM annotation_history",
            [],
            |r| r.get(0),
        )?;
        self.0.set(Some(g));
        Ok(g)
    }
}

/// Journals what changes from the stored row of `b` to `b`. Does nothing for a book that is not
/// stored yet (nothing existed before) and nothing when the annotations did not change.
pub(crate) fn record_changes(
    conn: &Connection,
    b: &Book,
    source: &Source,
    group: &GroupId,
    now: DateTime<Utc>,
) -> Result<()> {
    let Ok(id) = i64::try_from(b.id) else {
        return Ok(());
    };
    let old: Option<String> = conn
        .prepare_cached("SELECT data FROM books WHERE id = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?;
    let Some(old) = old else {
        return Ok(());
    };
    if b.highlights.is_empty()
        && b.bookmarks.is_empty()
        && !old.contains("\"highlights\":[{")
        && !old.contains("\"bookmarks\":[{")
    {
        return Ok(());
    }
    let Ok(old) = serde_json::from_str::<Annotations>(&old) else {
        return Ok(());
    };
    let changes = diff(
        &Snap {
            highlights: old.highlights,
            bookmarks: old.bookmarks,
        },
        &Snap::of(b),
    );
    if changes.is_empty() {
        return Ok(());
    }
    let g = group.get(conn)?;
    let authors = b.meta.authors_display();
    let mut ins = conn.prepare_cached(
        "INSERT INTO annotation_history(at, book_id, title, authors, kind, action, source,
                                        group_id, annotation_id, record)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    let src = source.as_db();
    for (action, rec) in changes {
        let json = serde_json::to_string(&rec)?;
        if json.len() > MAX_RECORD_BYTES {
            continue;
        }
        ins.execute(params![
            now.timestamp_millis(),
            id,
            b.meta.title,
            authors,
            rec.kind().as_str(),
            action.as_str(),
            src,
            g,
            rec.id(),
            json
        ])?;
    }
    drop(ins);
    prune_if_due(conn, now)?;
    Ok(())
}

fn prune_if_due(conn: &Connection, now: DateTime<Utc>) -> Result<()> {
    let last: Option<i64> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'history_pruned'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let now_ms = now.timestamp_millis();
    if last.is_some_and(|l| now_ms - l < PRUNE_EVERY_MS && l <= now_ms) {
        return Ok(());
    }
    prune(conn, now)?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES('history_pruned', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [now_ms],
    )?;
    Ok(())
}

/// The retention rule, with the caps: returns the number of rows removed.
pub(crate) fn prune(conn: &Connection, now: DateTime<Utc>) -> Result<usize> {
    let cutoff = (now - chrono::Duration::days(KEEP_DAYS)).timestamp_millis();
    let mut gone = conn.execute(
        "DELETE FROM annotation_history WHERE at < ?1 AND id NOT IN
           (SELECT id FROM annotation_history ORDER BY id DESC LIMIT ?2)",
        params![cutoff, KEEP_MIN as i64],
    )?;
    // hard caps: the newest MAX_ENTRIES rows, and the newest rows that fit MAX_BYTES of text
    let by_count: Option<i64> = conn
        .query_row(
            "SELECT id FROM annotation_history ORDER BY id DESC LIMIT 1 OFFSET ?1",
            [MAX_ENTRIES as i64],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = by_count {
        gone += conn.execute("DELETE FROM annotation_history WHERE id <= ?1", [id])?;
    }
    let total: i64 = conn.query_row(
        "SELECT coalesce(sum(length(record)), 0) FROM annotation_history",
        [],
        |r| r.get(0),
    )?;
    if total > MAX_BYTES as i64 {
        let by_bytes: Option<i64> = conn
            .query_row(
                "SELECT id FROM (SELECT id, sum(length(record)) OVER (ORDER BY id DESC) AS s
                                 FROM annotation_history)
                 WHERE s > ?1 ORDER BY id DESC LIMIT 1",
                [MAX_BYTES as i64],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = by_bytes {
            gone += conn.execute("DELETE FROM annotation_history WHERE id <= ?1", [id])?;
        }
    }
    Ok(gone)
}

/// Runs the retention pass when the database opens (and it is due).
pub(crate) fn on_open(conn: &Connection) {
    let _ = prune_if_due(conn, Utc::now());
}

const COLS: &str = "id, at, book_id, title, kind, action, source, authors, group_id, record";

impl SqliteStore {
    /// The newest `limit` entries that match `filter`, **oldest first** (the order
    /// `bookx_core::annotation_journal::group_by_day` expects).
    pub fn history(&self, filter: &Filter, limit: usize) -> Result<Vec<Entry>> {
        use bookx_core::annotation_journal::ActionFilter;
        let conn = self.lock();
        let mut sql = format!("SELECT {COLS} FROM annotation_history WHERE 1 = 1");
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        match filter.action {
            ActionFilter::All => {}
            ActionFilter::Deleted => sql.push_str(" AND action = 'deleted'"),
            ActionFilter::Edited => sql.push_str(" AND action = 'edited'"),
        }
        if let Some(s) = &filter.source {
            args.push(s.as_db().into());
            sql.push_str(&format!(" AND source = ?{}", args.len()));
        }
        if let Some(b) = filter.book {
            args.push((b as i64).into());
            sql.push_str(&format!(" AND book_id = ?{}", args.len()));
        }
        sql.push_str(" ORDER BY id DESC");
        let mut st = conn.prepare(&sql)?;
        let mut rows = st.query(rusqlite::params_from_iter(args))?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let Some(e) = entry_of_cols(r)? else {
                continue;
            };
            if filter.matches(&e) {
                out.push(e);
                if out.len() >= limit {
                    break;
                }
            }
        }
        out.reverse();
        Ok(out)
    }

    /// The entries with these ids (in the order of `ids`; unknown ones are left out).
    pub fn history_entries(&self, ids: &[u64]) -> Result<Vec<Entry>> {
        let conn = self.lock();
        let mut st = conn.prepare_cached(&format!(
            "SELECT {COLS} FROM annotation_history WHERE id = ?1"
        ))?;
        let mut out = Vec::new();
        for id in ids {
            let e = st
                .query_row([*id as i64], entry_of_cols)
                .optional()?
                .flatten();
            out.extend(e);
        }
        Ok(out)
    }

    /// The entries a save recorded: those of `group` (newest group when `None`).
    pub fn history_group(&self, group: u64) -> Result<Vec<Entry>> {
        let conn = self.lock();
        let mut st = conn.prepare_cached(&format!(
            "SELECT {COLS} FROM annotation_history WHERE group_id = ?1 ORDER BY id"
        ))?;
        let rows = st.query_map([group as i64], entry_of_cols)?;
        let mut out = Vec::new();
        for r in rows {
            out.extend(r?);
        }
        Ok(out)
    }

    /// The newest group id (0 when the history is empty): ask before a save, read what it added
    /// afterwards with [`history_since`](Self::history_since).
    pub fn history_mark(&self) -> u64 {
        self.lock()
            .query_row(
                "SELECT coalesce(max(id), 0) FROM annotation_history",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64
    }

    /// Entries added after `mark` (see [`history_mark`](Self::history_mark)), oldest first.
    pub fn history_since(&self, mark: u64) -> Result<Vec<Entry>> {
        let conn = self.lock();
        let mut st = conn.prepare_cached(&format!(
            "SELECT {COLS} FROM annotation_history WHERE id > ?1 ORDER BY id"
        ))?;
        let rows = st.query_map([mark as i64], entry_of_cols)?;
        let mut out = Vec::new();
        for r in rows {
            out.extend(r?);
        }
        Ok(out)
    }

    /// Number of entries.
    pub fn history_count(&self) -> usize {
        self.lock()
            .query_row("SELECT count(*) FROM annotation_history", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize
    }

    /// The devices that appear in the history, for the filter.
    pub fn history_devices(&self) -> Vec<String> {
        let conn = self.lock();
        let Ok(mut st) = conn.prepare(
            "SELECT DISTINCT source FROM annotation_history WHERE source LIKE 'device:%' ORDER BY 1",
        ) else {
            return Vec::new();
        };
        st.query_map([], |r| r.get::<_, String>(0))
            .map(|rows| {
                rows.flatten()
                    .filter_map(|s| s.strip_prefix("device:").map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Applies the retention rule now; returns how many entries went.
    pub fn history_prune(&self, now: DateTime<Utc>) -> Result<usize> {
        prune(&self.lock(), now)
    }

    /// Forgets the whole history ("Clear History"). The file shrinks with
    /// [`history_compact`](Self::history_compact).
    pub fn history_clear(&self) -> Result<usize> {
        Ok(self.lock().execute("DELETE FROM annotation_history", [])?)
    }

    /// Gives the space of deleted entries back (VACUUM; only on the user's request).
    pub fn history_compact(&self) -> Result<()> {
        self.lock().execute_batch("VACUUM")?;
        Ok(())
    }
}

fn entry_of_cols(r: &rusqlite::Row<'_>) -> rusqlite::Result<Option<Entry>> {
    let record: String = r.get(9)?;
    let kind: String = r.get(4)?;
    let action: String = r.get(5)?;
    let source: String = r.get(6)?;
    let (Ok(record), Some(kind), Some(action)) = (
        serde_json::from_str::<Record>(&record),
        Kind::parse(&kind),
        Action::parse(&action),
    ) else {
        return Ok(None);
    };
    Ok(Some(Entry {
        id: r.get::<_, i64>(0)? as u64,
        at: DateTime::from_timestamp_millis(r.get(1)?).unwrap_or_default(),
        book: r.get::<_, i64>(2)? as u64,
        title: r.get(3)?,
        authors: r.get(7)?,
        group: r.get::<_, i64>(8)? as u64,
        kind,
        action,
        source: Source::from_db(&source),
        record,
    }))
}
