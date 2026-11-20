//! The Kobo library database (`.kobo/KoboReader.sqlite`): reading state and shelves.
//!
//! Read: every book's progress and status, and the user's shelves with the books on them.
//! Write: put sent books on a shelf (creating it), nothing else. Before the first change a copy
//! `KoboReader.sqlite.bookx-backup` is made, changes happen in one transaction, and a database
//! without the expected tables is left alone. The Kobo itself rescans the books folder when it is
//! unplugged and adds the `content` rows; shelf entries for books it has not scanned yet are
//! picked up then (this is what Calibre's Kobo driver relies on too).
//!
//! Reading never opens the database on the device itself: the Kobo keeps it in WAL mode, and even
//! a read-only connection touches the `-shm` file next to a WAL database. The files are copied
//! (database and `-wal`) into a private temporary folder and read there. See
//! `docs/kobo-real-device-report.md` for what was checked against a real Libra Colour (schema
//! version 176).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OpenFlags};

use crate::reading::{DeviceNote, DeviceState, Status};

/// Prefix the Kobo uses for files on its internal storage.
pub const ONBOARD: &str = "file:///mnt/onboard/";

/// `file:///mnt/onboard/<rel>` for a path below the device root (`/` separated, percent-free).
pub fn content_id(rel: &str) -> String {
    format!("{ONBOARD}{}", rel.trim_start_matches('/'))
}

/// Path below the device root of a Kobo content id, `None` for ids that are not files on the
/// internal storage (store books, SD card `file:///mnt/sd/`, chapters `…epub!OEBPS/x.html`).
pub fn rel_of_content_id(id: &str) -> Option<&str> {
    let rel = id.strip_prefix(ONBOARD)?;
    (!rel.is_empty() && !rel.contains('!')).then_some(rel)
}

/// A shelf and the content ids on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shelf {
    pub name: String,
    pub content_ids: Vec<String>,
}

/// A private copy of the database (and its write-ahead log) to read from.
struct Snapshot {
    dir: PathBuf,
    db: PathBuf,
}

impl Snapshot {
    fn of(db: &Path) -> Result<Snapshot> {
        let dir = std::env::temp_dir().join(format!("bookx-kobo-db-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir)?;
        let name = db.file_name().unwrap_or_default().to_os_string();
        let copy = dir.join(&name);
        let snap = Snapshot {
            dir,
            db: copy.clone(),
        };
        std::fs::copy(db, &copy).with_context(|| format!("reading {}", db.display()))?;
        let mut wal = name;
        wal.push("-wal");
        let src = db.with_file_name(&wal);
        if src.is_file() {
            // a log left behind holds changes that are not in the main file yet
            std::fs::copy(&src, snap.dir.join(&wal)).ok();
        }
        Ok(snap)
    }

    fn open(&self) -> Result<Connection> {
        let c = Connection::open_with_flags(
            &self.db,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("opening a copy of {}", self.db.display()))?;
        c.busy_timeout(Duration::from_secs(3))?;
        Ok(c)
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// Opens a private copy of `db` for reading (the original is only ever read from, never opened by
/// SQLite). The copy is gone when the returned guard and connection are dropped.
fn with_copy<T>(db: &Path, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let snap = Snapshot::of(db)?;
    let c = snap.open()?;
    let r = f(&c);
    drop(c);
    r
}

fn columns(c: &Connection, table: &str) -> Result<Vec<String>> {
    let mut st = c.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = st
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(cols)
}

fn require(c: &Connection, table: &str, needed: &[&str]) -> Result<Vec<String>> {
    let cols = columns(c, table)?;
    for n in needed {
        if !cols.iter().any(|c| c == n) {
            bail!("this does not look like a Kobo database (no {table}.{n})");
        }
    }
    Ok(cols)
}

fn is_true(v: Option<String>) -> bool {
    matches!(v.as_deref(), Some("true") | Some("1"))
}

/// Reading state of every book in the database (content type 6, not the chapter rows).
pub fn read_states(db: &Path) -> Result<Vec<(String, DeviceState)>> {
    with_copy(db, read_states_in)
}

/// Mime types of rows that are books. The Kobo also lists every image, text and script it finds
/// below `.adds` (KOReader's thousands of files) as content type 6.
pub fn is_book_mime(mime: &str) -> bool {
    let m = mime.to_ascii_lowercase();
    m.is_empty()
        || matches!(
            m.as_str(),
            "application/epub+zip"
                | "application/x-kobo-epub+zip"
                | "application/pdf"
                | "application/x-mobipocket-ebook"
                | "application/x-cbz"
                | "application/vnd.comicbook+zip"
                | "application/x-fb2+zip"
                | "application/octet-stream"
        )
}

fn read_states_in(c: &Connection) -> Result<Vec<(String, DeviceState)>> {
    let cols = require(c, "content", &["ContentID", "ContentType", "Title"])?;
    let has = |n: &str| cols.iter().any(|c| c == n);
    let sql = format!(
        "SELECT ContentID, Title, {}, {}, {}, {} FROM content WHERE ContentType = '6' OR ContentType = 6",
        if has("___PercentRead") { "___PercentRead" } else { "0" },
        if has("ReadStatus") { "ReadStatus" } else { "0" },
        if has("DateLastRead") { "DateLastRead" } else { "NULL" },
        if has("MimeType") { "MimeType" } else { "NULL" },
    );
    let mut st = c.prepare(&sql)?;
    let rows = st.query_map([], |r| {
        let id: String = r.get(0)?;
        let title: Option<String> = r.get(1)?;
        let pct: f64 = r.get::<_, Option<f64>>(2)?.unwrap_or(0.0);
        let status: i64 = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
        let last: Option<String> = r.get(4)?;
        let mime: Option<String> = r.get(5)?;
        Ok((id, title, pct, status, last, mime))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, title, pct, status, last, mime) = row?;
        if !is_book_mime(mime.as_deref().unwrap_or("")) {
            continue;
        }
        let status = match status {
            2 => Status::Finished,
            1 => Status::Reading,
            _ if pct > 0.0 => Status::Reading,
            _ => Status::Unread,
        };
        out.push((
            id,
            DeviceState {
                title,
                progress: (pct / 100.0).clamp(0.0, 1.0),
                status,
                last_read: last.filter(|l| !l.trim().is_empty()),
                source: "Kobo database",
            },
        ));
    }
    Ok(out)
}

/// The user's shelves with their books (deleted shelves and entries are left out).
pub fn read_shelves(db: &Path) -> Result<Vec<Shelf>> {
    with_copy(db, read_shelves_in)
}

fn read_shelves_in(c: &Connection) -> Result<Vec<Shelf>> {
    let shelf_cols = require(c, "Shelf", &["Name"])?;
    require(c, "ShelfContent", &["ShelfName", "ContentId"])?;
    let internal = shelf_cols.iter().any(|c| c == "InternalName");
    let deleted = shelf_cols.iter().any(|c| c == "_IsDeleted");
    let sql = format!(
        "SELECT Name, {}, {} FROM Shelf",
        if internal { "InternalName" } else { "Name" },
        if deleted { "_IsDeleted" } else { "NULL" }
    );
    let mut st = c.prepare(&sql)?;
    let shelves = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let sc_cols = columns(c, "ShelfContent")?;
    let sc_deleted = sc_cols.iter().any(|c| c == "_IsDeleted");
    let mut out = Vec::new();
    for (name, internal_name, del) in shelves {
        if is_true(del) {
            continue;
        }
        let sql = format!(
            "SELECT ContentId, {} FROM ShelfContent WHERE ShelfName = ?1 ORDER BY ContentId",
            if sc_deleted { "_IsDeleted" } else { "NULL" }
        );
        let mut st = c.prepare(&sql)?;
        let ids = st
            .query_map([&internal_name], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        out.push(Shelf {
            name,
            content_ids: ids
                .into_iter()
                .filter(|(_, d)| !is_true(d.clone()))
                .map(|(i, _)| i)
                .collect(),
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// Puts the books on the shelf `shelf`, creating it when it does not exist and re-activating
/// entries that were deleted. `now` is the time stamp written (`2026-11-12T10:00:00Z`).
/// Returns how many entries were added or revived.
pub fn add_to_shelf(db: &Path, shelf: &str, content_ids: &[String], now: &str) -> Result<usize> {
    let shelf = shelf.trim();
    if shelf.is_empty() || shelf.len() > 100 || shelf.chars().any(|c| c.is_control()) {
        bail!("not a usable shelf name");
    }
    if content_ids.is_empty() {
        return Ok(0);
    }
    with_copy(db, |c| {
        require(c, "Shelf", &["Name", "Id", "InternalName", "_IsDeleted"])?;
        require(c, "ShelfContent", &["ShelfName", "ContentId"])?;
        Ok(())
    })?;
    backup_once(db)?;
    let mut c = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("opening {} for writing", db.display()))?;
    c.busy_timeout(Duration::from_secs(5))?;
    let shelf_cols = columns(&c, "Shelf")?;
    let sc_cols = columns(&c, "ShelfContent")?;
    let tx = c.transaction()?;

    let existing: Option<String> = tx
        .query_row(
            "SELECT InternalName FROM Shelf WHERE Name = ?1 AND COALESCE(_IsDeleted,'false') NOT IN ('true','1')",
            [shelf],
            |r| r.get(0),
        )
        .ok();
    let internal = match existing {
        Some(i) => i,
        None => {
            let has = |n: &str| shelf_cols.iter().any(|c| c == n);
            let mut names = vec!["Id", "Name"];
            let mut vals: Vec<String> = vec![uuid::Uuid::new_v4().to_string(), shelf.to_string()];
            for (col, val) in [
                ("InternalName", shelf),
                ("CreationDate", now),
                ("LastModified", now),
                ("Type", "UserTag"),
                ("_IsDeleted", "false"),
                ("_IsVisible", "true"),
                ("_IsSynced", "false"),
            ] {
                if has(col) {
                    names.push(col);
                    vals.push(val.to_string());
                }
            }
            let marks = (1..=names.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            tx.execute(
                &format!("INSERT INTO Shelf ({}) VALUES ({marks})", names.join(",")),
                rusqlite::params_from_iter(vals.iter()),
            )?;
            shelf.to_string()
        }
    };

    let mut changed = 0;
    for id in content_ids {
        let present: Option<Option<String>> = if sc_cols.iter().any(|c| c == "_IsDeleted") {
            tx.query_row(
                "SELECT _IsDeleted FROM ShelfContent WHERE ShelfName = ?1 AND ContentId = ?2",
                params![internal, id],
                |r| r.get(0),
            )
            .ok()
        } else {
            tx.query_row(
                "SELECT NULL FROM ShelfContent WHERE ShelfName = ?1 AND ContentId = ?2",
                params![internal, id],
                |r| r.get(0),
            )
            .ok()
        };
        match present {
            Some(del) if !is_true(del.clone()) => {}
            Some(_) => {
                tx.execute(
                    "UPDATE ShelfContent SET _IsDeleted = 'false', _IsSynced = 'false' WHERE ShelfName = ?1 AND ContentId = ?2",
                    params![internal, id],
                )?;
                changed += 1;
            }
            None => {
                let has = |n: &str| sc_cols.iter().any(|c| c == n);
                let mut names = vec!["ShelfName", "ContentId"];
                let mut vals = vec![internal.clone(), id.clone()];
                for (col, val) in [
                    ("DateModified", now),
                    ("_IsDeleted", "false"),
                    ("_IsSynced", "false"),
                ] {
                    if has(col) {
                        names.push(col);
                        vals.push(val.to_string());
                    }
                }
                let marks = (1..=names.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(",");
                tx.execute(
                    &format!(
                        "INSERT INTO ShelfContent ({}) VALUES ({marks})",
                        names.join(",")
                    ),
                    rusqlite::params_from_iter(vals.iter()),
                )?;
                changed += 1;
            }
        }
    }
    tx.commit()?;
    Ok(changed)
}

/// Keeps a consistent copy of the database next to it before the first change. `VACUUM INTO`
/// includes whatever is still in the write-ahead log; a plain file copy would not.
fn backup_once(db: &Path) -> Result<()> {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push(".bookx-backup");
    let bak: PathBuf = db.with_file_name(name);
    if bak.exists() {
        return Ok(());
    }
    let snap = Snapshot::of(db)?;
    let c = snap.open()?;
    c.execute("VACUUM INTO ?1", [bak.to_string_lossy().as_ref()])
        .with_context(|| format!("backing up to {}", bak.display()))?;
    Ok(())
}

// -- device identity and database facts ---------------------------------------------------------

/// What `.kobo/version` says: `serial,kernel,firmware,kernel,kernel,model-id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KoboVersion {
    pub firmware: String,
    /// The last field, `00000000-0000-0000-0000-000000000390`.
    pub model_id: String,
    pub model: Option<&'static str>,
}

impl KoboVersion {
    /// "Kobo Libra Colour, firmware 4.46.23852". The serial number is never kept.
    pub fn describe(&self) -> String {
        format!(
            "{}, firmware {}",
            self.model
                .map(|m| format!("Kobo {m}"))
                .unwrap_or_else(|| format!("Kobo (model id {})", self.model_id)),
            self.firmware
        )
    }
}

/// Models whose id has been seen on a real device. Others show their id instead of a guess.
fn model_of(id: &str) -> Option<&'static str> {
    match id.rsplit('-').next()? {
        "000000000390" => Some("Libra Colour"),
        _ => None,
    }
}

/// Parses the text of `.kobo/version`.
pub fn parse_version(text: &str) -> Option<KoboVersion> {
    let f: Vec<&str> = text.trim().split(',').map(str::trim).collect();
    if f.len() < 6 || f[0].is_empty() || f[2].is_empty() {
        return None;
    }
    let looks_like_version =
        f[2].split('.').count() >= 2 && f[2].chars().all(|c| c.is_ascii_digit() || c == '.');
    looks_like_version.then(|| KoboVersion {
        firmware: f[2].to_string(),
        model_id: f[5].to_string(),
        model: model_of(f[5]),
    })
}

/// Facts about the library database, read from a private copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbInfo {
    /// `DbVersion.version`, the schema version the firmware migrates by.
    pub schema_version: Option<i64>,
    /// The database is in write-ahead-log mode (the file header says so).
    pub wal: bool,
    /// Rows of content type 6 that are books (see [`is_book_mime`]).
    pub book_rows: usize,
    /// Rows of content type 6 that are not (images and scripts below `.adds`).
    pub other_rows: usize,
    pub shelves: usize,
    pub highlights: usize,
}

/// Reads [`DbInfo`] from a copy of the database.
pub fn db_info(db: &Path) -> Result<DbInfo> {
    let mut head = [0u8; 20];
    {
        use std::io::Read;
        std::fs::File::open(db)?.read_exact(&mut head)?;
    }
    let wal = head[18] == 2 && head[19] == 2;
    with_copy(db, |c| {
        let schema_version = c
            .query_row("SELECT MAX(version) FROM DbVersion", [], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .ok()
            .flatten();
        let book_rows = read_states_in(c)?.len();
        let total: i64 = c.query_row(
            "SELECT COUNT(*) FROM content WHERE ContentType = '6' OR ContentType = 6",
            [],
            |r| r.get(0),
        )?;
        let other_rows = (total as usize).saturating_sub(book_rows);
        let shelves = read_shelves_in(c).map(|s| s.len()).unwrap_or(0);
        let highlights = read_notes_in(c).map(|n| n.len()).unwrap_or(0);
        Ok(DbInfo {
            schema_version,
            wal,
            book_rows,
            other_rows,
            shelves,
            highlights,
        })
    })
}

/// Highlights and notes the reader made on the Kobo: `(content id of the book, note)`.
pub fn read_notes(db: &Path) -> Result<Vec<(String, DeviceNote)>> {
    with_copy(db, read_notes_in)
}

fn read_notes_in(c: &Connection) -> Result<Vec<(String, DeviceNote)>> {
    let cols = require(c, "Bookmark", &["VolumeID", "Text"])?;
    let has = |n: &str| cols.iter().any(|c| c == n);
    let sql = format!(
        "SELECT VolumeID, Text, {}, {}, {}, {} FROM Bookmark WHERE {} {}",
        if has("Annotation") {
            "Annotation"
        } else {
            "NULL"
        },
        if has("DateCreated") {
            "DateCreated"
        } else {
            "NULL"
        },
        if has("DateModified") {
            "DateModified"
        } else {
            "NULL"
        },
        if has("Color") { "Color" } else { "NULL" },
        if has("Hidden") {
            "COALESCE(Hidden, 0) = 0"
        } else {
            "1"
        },
        if has("Type") {
            "AND Type IN ('highlight', 'note')"
        } else {
            ""
        },
    );
    let mut st = c.prepare(&sql)?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for (vol, text, note, created, modified, color) in rows {
        let text = text.unwrap_or_default();
        let note = note.unwrap_or_default();
        if text.trim().is_empty() && note.trim().is_empty() {
            continue;
        }
        let updated = modified.filter(|m| Some(m) != created.as_ref());
        out.push((
            vol,
            DeviceNote {
                text,
                note,
                chapter: None,
                created,
                updated,
                color: color
                    .and_then(bookx_core::devstyle::kobo_color)
                    .map(str::to_string),
                style: None,
                bookmark: false,
            },
        ));
    }
    Ok(out)
}

/// A highlight or note together with the book it belongs to, as the Kobo database names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookNote {
    pub content_id: String,
    pub title: String,
    pub author: Option<String>,
    pub note: DeviceNote,
}

/// Highlights and notes with the title and author of their books, for importing from a device or
/// from a copied `KoboReader.sqlite` (read from a private copy, the original is not opened). A
/// book without a `content` row is named after its file.
pub fn read_book_notes(db: &Path) -> Result<Vec<BookNote>> {
    with_copy(db, |c| {
        let notes = read_notes_in(c)?;
        let mut titles: std::collections::HashMap<String, (String, Option<String>)> =
            Default::default();
        if let Ok(mut st) = c.prepare("SELECT ContentID, Title, Attribution FROM content") {
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })?;
            for (id, t, a) in rows.flatten() {
                if let Some(t) = t.filter(|t| !t.trim().is_empty()) {
                    titles.insert(id, (t, a.filter(|a| !a.trim().is_empty())));
                }
            }
        }
        Ok(notes
            .into_iter()
            .map(|(id, note)| {
                let (title, author) = titles.get(&id).cloned().unwrap_or_else(|| {
                    let name = id.rsplit('/').next().unwrap_or(&id);
                    (crate::devfs::file_stem(name).to_string(), None)
                });
                BookNote {
                    content_id: id,
                    title,
                    author,
                    note,
                }
            })
            .collect())
    })
}

/// Adds a book row and one highlight to a database made by [`create_real_schema_database`]
/// (tests of other crates).
#[doc(hidden)]
pub fn add_test_highlight(
    db: &Path,
    content_id: &str,
    title: &str,
    author: &str,
    text: &str,
    note: &str,
) -> Result<()> {
    let c = Connection::open(db)?;
    c.execute(
        "INSERT INTO content (ContentID, ContentType, MimeType, Title, Attribution, ___UserID) VALUES (?1,'6','application/epub+zip',?2,?3,'')",
        params![content_id, title, author],
    )?;
    c.execute(
        "INSERT INTO Bookmark (BookmarkID, VolumeID, ContentID, StartContainerPath, StartContainerChildIndex, StartOffset, EndContainerPath, EndContainerChildIndex, EndOffset, Text, Annotation, DateCreated, Type) VALUES (?1,?2,'c','p',0,0,'p',0,5,?3,?4,'2026-11-01T08:00:00Z','highlight')",
        params![uuid::Uuid::new_v4().to_string(), content_id, text, note],
    )?;
    Ok(())
}

/// Sets `Bookmark.Color` of every row (tests of other crates).
#[doc(hidden)]
pub fn set_test_color_all(db: &Path, color: i64) -> Result<()> {
    Connection::open(db)?.execute("UPDATE Bookmark SET Color = ?1", params![color])?;
    Ok(())
}

/// Adds one more highlight with a colour (`Bookmark.Color`) and a type (`highlight`, `note` or
/// `dogear`) for a book that [`add_test_highlight`] added (tests of other crates).
#[doc(hidden)]
pub fn add_test_highlight_colored(
    db: &Path,
    content_id: &str,
    text: &str,
    note: &str,
    color: i64,
    kind: &str,
) -> Result<()> {
    let c = Connection::open(db)?;
    c.execute(
        "INSERT INTO Bookmark (BookmarkID, VolumeID, ContentID, StartContainerPath, StartContainerChildIndex, StartOffset, EndContainerPath, EndContainerChildIndex, EndOffset, Text, Annotation, DateCreated, Type, Color) VALUES (?1,?2,'c','span#kobo.1.1',0,17,'span#kobo.1.9',0,60,?3,?4,'2026-11-01T08:00:00Z',?6,?5)",
        params![uuid::Uuid::new_v4().to_string(), content_id, text, note, color, kind],
    )?;
    Ok(())
}

/// Creates a database with the Kobo tables used here (tests of this and other crates).
#[doc(hidden)]
pub fn create_test_database(db: &Path) -> Result<()> {
    let c = Connection::open(db)?;
    c.execute_batch(
        "CREATE TABLE content (ContentID TEXT NOT NULL, ContentType TEXT NOT NULL, Title TEXT, Attribution TEXT, ___PercentRead INTEGER, ReadStatus INTEGER, DateLastRead TEXT, PRIMARY KEY(ContentID));
         CREATE TABLE Shelf (CreationDate TEXT, Id TEXT, InternalName TEXT, LastModified TEXT, Name TEXT, Type TEXT, _IsDeleted BOOL, _IsVisible BOOL, _IsSynced BOOL, PRIMARY KEY(Id));
         CREATE TABLE ShelfContent (ShelfName TEXT, ContentId TEXT, DateModified TEXT, _IsDeleted BOOL, _IsSynced BOOL, PRIMARY KEY(ShelfName, ContentId));",
    )?;
    Ok(())
}

/// The table definitions of a real Kobo Libra Colour database (schema version 176, no data),
/// taken from the device: `content`, `Shelf`, `ShelfContent`, `Bookmark`, `Event`, `DbVersion`.
#[doc(hidden)]
pub const REAL_SCHEMA_176: &str = include_str!("../tests/fixtures/kobo-schema-176.sql");

/// Creates a database with the real schema, in WAL mode like the device's, and `DbVersion` 176.
#[doc(hidden)]
pub fn create_real_schema_database(db: &Path) -> Result<()> {
    let c = Connection::open(db)?;
    c.execute_batch("PRAGMA journal_mode=WAL;")?;
    c.execute_batch(REAL_SCHEMA_176)?;
    c.execute_batch("INSERT INTO DbVersion VALUES (176);")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            let p = std::env::temp_dir().join(format!("bookx-kobo-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn db(&self) -> PathBuf {
            let p = self.0.join("KoboReader.sqlite");
            create_test_database(&p).unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn sql(db: &Path, q: &str) {
        Connection::open(db).unwrap().execute_batch(q).unwrap();
    }

    #[test]
    fn content_ids_map_to_device_paths() {
        assert_eq!(
            content_id("Bookx/A b.kepub.epub"),
            "file:///mnt/onboard/Bookx/A b.kepub.epub"
        );
        assert_eq!(
            rel_of_content_id("file:///mnt/onboard/Bookx/x.epub"),
            Some("Bookx/x.epub")
        );
        assert_eq!(
            rel_of_content_id("file:///mnt/onboard/x.epub!OEBPS/c.html"),
            None
        );
        assert_eq!(rel_of_content_id("file:///mnt/sd/x.epub"), None);
        assert_eq!(rel_of_content_id("0123-4567"), None);
    }

    #[test]
    fn reading_state_comes_from_the_content_table() {
        let t = Tmp::new();
        let db = t.db();
        sql(
            &db,
            "INSERT INTO content VALUES ('file:///mnt/onboard/Bookx/A.kepub.epub','6','Alpha','Ann',40,1,'2026-11-01T10:00:00Z');
             INSERT INTO content VALUES ('file:///mnt/onboard/B.epub','6','Beta','Bob',100,2,'2026-11-02T10:00:00Z');
             INSERT INTO content VALUES ('file:///mnt/onboard/C.epub','6','Gamma','Cy',0,0,NULL);
             INSERT INTO content VALUES ('file:///mnt/onboard/A.epub!OEBPS/c1.html','9','chapter','Ann',0,0,NULL);",
        );
        let s = read_states(&db).unwrap();
        assert_eq!(s.len(), 3, "chapters are not books");
        let get = |id: &str| s.iter().find(|(i, _)| i.ends_with(id)).unwrap().1.clone();
        let a = get("A.kepub.epub");
        assert_eq!(a.status, Status::Reading);
        assert!((a.progress - 0.4).abs() < 1e-9);
        assert_eq!(a.title.as_deref(), Some("Alpha"));
        assert_eq!(get("B.epub").status, Status::Finished);
        assert_eq!(get("B.epub").progress, 1.0);
        assert_eq!(get("C.epub").status, Status::Unread);
    }

    #[test]
    fn shelves_list_their_books_and_hide_deleted_ones() {
        let t = Tmp::new();
        let db = t.db();
        sql(
            &db,
            "INSERT INTO Shelf VALUES ('d','1','Sci-fi','d','Sci-fi','UserTag','false','true','false');
             INSERT INTO Shelf VALUES ('d','2','Gone','d','Gone','UserTag','true','true','false');
             INSERT INTO ShelfContent VALUES ('Sci-fi','file:///mnt/onboard/A.epub','d','false','false');
             INSERT INTO ShelfContent VALUES ('Sci-fi','file:///mnt/onboard/B.epub','d','true','false');",
        );
        let s = read_shelves(&db).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "Sci-fi");
        assert_eq!(s[0].content_ids, vec!["file:///mnt/onboard/A.epub"]);
    }

    #[test]
    fn sent_books_are_put_on_a_new_shelf_and_a_backup_is_made() {
        let t = Tmp::new();
        let db = t.db();
        let ids = vec![
            content_id("Bookx/One.kepub.epub"),
            content_id("Bookx/Two.kepub.epub"),
        ];
        assert_eq!(
            add_to_shelf(&db, "Bookx", &ids, "2026-11-12T10:00:00Z").unwrap(),
            2
        );
        assert!(t.0.join("KoboReader.sqlite.bookx-backup").is_file());
        let s = read_shelves(&db).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "Bookx");
        assert_eq!(s[0].content_ids.len(), 2);
        // doing it again adds nothing and does not duplicate the shelf
        assert_eq!(
            add_to_shelf(&db, "Bookx", &ids, "2026-11-12T11:00:00Z").unwrap(),
            0
        );
        assert_eq!(read_shelves(&db).unwrap().len(), 1);
        // the new shelf row carries the fields the Kobo expects
        let c = Connection::open(&db).unwrap();
        let (ty, vis, del): (String, String, String) = c
            .query_row(
                "SELECT Type, _IsVisible, _IsDeleted FROM Shelf WHERE Name='Bookx'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (ty.as_str(), vis.as_str(), del.as_str()),
            ("UserTag", "true", "false")
        );
    }

    #[test]
    fn an_existing_shelf_is_reused_and_deleted_entries_come_back() {
        let t = Tmp::new();
        let db = t.db();
        sql(
            &db,
            "INSERT INTO Shelf VALUES ('d','1','Fav','d','Favourites','UserTag','false','true','true');
             INSERT INTO ShelfContent VALUES ('Fav','file:///mnt/onboard/A.epub','d','true','true');",
        );
        let n = add_to_shelf(
            &db,
            "Favourites",
            &[
                "file:///mnt/onboard/A.epub".into(),
                "file:///mnt/onboard/B.epub".into(),
            ],
            "2026-11-12T10:00:00Z",
        )
        .unwrap();
        assert_eq!(n, 2);
        let s = read_shelves(&db).unwrap();
        assert_eq!(s.len(), 1, "no second shelf with the same name");
        assert_eq!(s[0].content_ids.len(), 2);
    }

    #[test]
    fn something_that_is_not_a_kobo_database_is_left_untouched() {
        let t = Tmp::new();
        let db = t.0.join("other.sqlite");
        sql(&db, "CREATE TABLE unrelated (x INTEGER);");
        let before = std::fs::read(&db).unwrap();
        assert!(add_to_shelf(&db, "Bookx", &["file:///mnt/onboard/A.epub".into()], "t").is_err());
        assert!(read_states(&db).is_err());
        assert!(read_shelves(&db).is_err());
        assert_eq!(std::fs::read(&db).unwrap(), before);
        assert!(!t.0.join("other.sqlite.bookx-backup").exists());
        assert!(read_states(&t.0.join("missing.sqlite")).is_err());
    }

    #[test]
    fn bad_shelf_names_are_refused() {
        let t = Tmp::new();
        let db = t.db();
        let id = vec!["file:///mnt/onboard/A.epub".to_string()];
        assert!(add_to_shelf(&db, "  ", &id, "t").is_err());
        assert!(add_to_shelf(&db, "a\nb", &id, "t").is_err());
        assert_eq!(add_to_shelf(&db, "ok", &[], "t").unwrap(), 0);
        // quotes are data, not SQL
        assert_eq!(
            add_to_shelf(&db, "It's \"x\"; DROP TABLE Shelf;--", &id, "t").unwrap(),
            1
        );
        assert_eq!(read_shelves(&db).unwrap().len(), 1);
    }

    // -- real schema -----------------------------------------------------------------------------

    /// Rows shaped like the ones found on a real device (titles and ids made up).
    const REAL_ROWS: &str = "
        INSERT INTO content (ContentID, ContentType, MimeType, Title, Attribution, ReadStatus, ___PercentRead, DateLastRead, IsDownloaded, ___UserID, FirstTimeReading)
          VALUES ('file:///mnt/onboard/Sideloaded Book.epub', '6', 'application/epub+zip', 'Sideloaded Book', 'Ann Author', 1, 54, '2026-08-11T13:22:26Z', 'true', '', 'false');
        INSERT INTO content (ContentID, ContentType, MimeType, Title, Attribution, ReadStatus, ___PercentRead, IsDownloaded, ___UserID)
          VALUES ('file:///mnt/onboard/Bookx/Sent.kepub.epub', '6', 'application/x-kobo-epub+zip', 'Sent', 'Bob', 2, 100, 'true', '');
        INSERT INTO content (ContentID, ContentType, MimeType, Title, ReadStatus, ___PercentRead, IsDownloaded, ___UserID)
          VALUES ('file:///mnt/onboard/.adds/koreader/resources/icons/x.svg', '6', 'image/svg+xml', 'x', 0, 0, 'true', '');
        INSERT INTO content (ContentID, ContentType, MimeType, Title, ReadStatus, ___PercentRead, IsDownloaded, ___UserID)
          VALUES ('file:///mnt/onboard/.adds/koreader/COPYING.txt', '6', 'text/plain', 'COPYING', 0, 0, 'true', '');
        INSERT INTO content (ContentID, ContentType, MimeType, Title, ReadStatus, ___PercentRead, IsDownloaded, ___UserID)
          VALUES ('d857113b-2f42-475d-83ae-42f86277d9df', '6', 'application/x-kobo-epub+zip', 'Store preview', 0, 0, 'false', 'u');
        INSERT INTO content (ContentID, ContentType, MimeType, Title, ReadStatus, ___PercentRead, IsDownloaded, ___UserID)
          VALUES ('file:///mnt/onboard/Sideloaded Book.epub#(0)OEBPS/c1.html', '9', 'application/epub+zip', 'c1', 0, 0, 'true', '');
        INSERT INTO Bookmark (BookmarkID, VolumeID, ContentID, StartContainerPath, StartContainerChildIndex, StartOffset, EndContainerPath, EndContainerChildIndex, EndOffset, Text, Annotation, DateCreated, DateModified, Type, Hidden)
          VALUES ('b1', 'file:///mnt/onboard/Bookx/Sent.kepub.epub', 'file:///mnt/onboard/Bookx/Sent.kepub.epub!c1', 'p', 0, 0, 'p', 0, 9, 'a highlighted line', 'my thought', '2026-11-01T08:00:00Z', '2026-11-02T08:00:00Z', 'highlight', 0);
        INSERT INTO Bookmark (BookmarkID, VolumeID, ContentID, StartContainerPath, StartContainerChildIndex, StartOffset, EndContainerPath, EndContainerChildIndex, EndOffset, Text, DateCreated, DateModified, Type, Hidden)
          VALUES ('b2', 'file:///mnt/onboard/Bookx/Sent.kepub.epub', 'c', 'p', 0, 0, 'p', 0, 9, 'dog ear', '2026-11-01T08:00:00Z', '2026-11-01T08:00:00Z', 'dogear', 0);
        INSERT INTO Bookmark (BookmarkID, VolumeID, ContentID, StartContainerPath, StartContainerChildIndex, StartOffset, EndContainerPath, EndContainerChildIndex, EndOffset, Text, DateCreated, DateModified, Type, Hidden)
          VALUES ('b3', 'file:///mnt/onboard/Bookx/Sent.kepub.epub', 'c', 'p', 0, 0, 'p', 0, 9, 'hidden one', '2026-11-01T08:00:00Z', '2026-11-01T08:00:00Z', 'highlight', 1);
    ";

    fn real_db(t: &Tmp) -> PathBuf {
        let p = t.0.join("KoboReader.sqlite");
        create_real_schema_database(&p).unwrap();
        sql(&p, REAL_ROWS);
        p
    }

    #[test]
    fn the_real_schema_is_the_one_from_the_device() {
        let t = Tmp::new();
        let db = real_db(&t);
        let c = Connection::open(&db).unwrap();
        let n: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('content')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n > 80, "the real content table has {n} columns");
        for tbl in ["Shelf", "ShelfContent", "Bookmark", "Event", "DbVersion"] {
            let k: i64 = c
                .query_row(
                    &format!("SELECT COUNT(*) FROM pragma_table_info('{tbl}')"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(k > 0, "{tbl}");
        }
    }

    #[test]
    fn real_rows_give_books_not_the_noise_below_adds() {
        let t = Tmp::new();
        let db = real_db(&t);
        let s = read_states(&db).unwrap();
        let ids: Vec<&str> = s.iter().map(|(i, _)| i.as_str()).collect();
        assert!(ids.contains(&"file:///mnt/onboard/Sideloaded Book.epub"));
        assert!(ids.contains(&"file:///mnt/onboard/Bookx/Sent.kepub.epub"));
        assert!(
            !ids.iter()
                .any(|i| i.ends_with(".svg") || i.ends_with(".txt")),
            "{ids:?}"
        );
        let sideloaded = &s.iter().find(|(i, _)| i.contains("Sideloaded")).unwrap().1;
        assert_eq!(sideloaded.status, Status::Reading);
        assert!((sideloaded.progress - 0.54).abs() < 1e-9);
        assert_eq!(
            sideloaded.last_read.as_deref(),
            Some("2026-08-11T13:22:26Z")
        );
        let sent = &s.iter().find(|(i, _)| i.contains("Sent")).unwrap().1;
        assert_eq!(sent.status, Status::Finished);
        // the store preview is a book row but not a file on the device
        assert!(s.iter().any(|(i, _)| i.starts_with("d857113b")));
        assert_eq!(
            rel_of_content_id("d857113b-2f42-475d-83ae-42f86277d9df"),
            None
        );
    }

    #[test]
    fn database_facts_come_from_the_real_tables() {
        let t = Tmp::new();
        let db = real_db(&t);
        let i = db_info(&db).unwrap();
        assert_eq!(i.schema_version, Some(176));
        assert_eq!(i.book_rows, 3);
        assert_eq!(i.other_rows, 2);
        assert_eq!(i.shelves, 0);
        assert_eq!(
            i.highlights, 1,
            "dog-ears and hidden rows are not highlights"
        );
    }

    #[test]
    fn highlights_and_notes_are_read_from_the_bookmark_table() {
        let t = Tmp::new();
        let db = real_db(&t);
        let n = read_notes(&db).unwrap();
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].0, "file:///mnt/onboard/Bookx/Sent.kepub.epub");
        assert_eq!(n[0].1.text, "a highlighted line");
        assert_eq!(n[0].1.note, "my thought");
        assert_eq!(n[0].1.created.as_deref(), Some("2026-11-01T08:00:00Z"));
        assert_eq!(n[0].1.updated.as_deref(), Some("2026-11-02T08:00:00Z"));
    }

    #[test]
    fn notes_come_with_the_title_and_author_of_their_book() {
        let t = Tmp::new();
        let db = real_db(&t);
        let n = read_book_notes(&db).unwrap();
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title, "Sent");
        assert_eq!(n[0].author.as_deref(), Some("Bob"));
        assert_eq!(n[0].note.text, "a highlighted line");
        // a book the content table does not know is named after its file
        sql(
            &db,
            "DELETE FROM content WHERE ContentID LIKE '%Sent.kepub.epub'",
        );
        let n = read_book_notes(&db).unwrap();
        assert_eq!(n[0].title, "Sent");
        assert_eq!(n[0].author, None);
    }

    #[test]
    fn reading_a_wal_database_leaves_the_folder_exactly_as_it_was() {
        let t = Tmp::new();
        let db = real_db(&t);
        // the state a cleanly ejected Kobo leaves: main file only (the device had -wal and -shm,
        // which a read-only open of the original would have rewritten)
        let before: Vec<_> = std::fs::read_dir(&t.0)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(before.len(), 1, "{before:?}");
        let bytes = std::fs::read(&db).unwrap();
        let _ = read_states(&db).unwrap();
        let _ = read_shelves(&db).unwrap();
        let _ = db_info(&db).unwrap();
        let _ = read_notes(&db).unwrap();
        let after: Vec<_> = std::fs::read_dir(&t.0)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(before, after, "no -shm/-wal/-journal appeared");
        assert_eq!(std::fs::read(&db).unwrap(), bytes);
        // and no private copy is left in the temp folder for this database
    }

    #[test]
    fn changes_still_in_the_wal_are_seen() {
        let t = Tmp::new();
        let db = t.0.join("KoboReader.sqlite");
        create_real_schema_database(&db).unwrap();
        let live = Connection::open(&db).unwrap();
        live.execute_batch("PRAGMA wal_autocheckpoint=0;").unwrap();
        live.execute_batch(REAL_ROWS).unwrap();
        // `live` is still open: the rows are in the -wal file, not in the main file
        assert!(t.0.join("KoboReader.sqlite-wal").metadata().unwrap().len() > 0);
        assert_eq!(read_states(&db).unwrap().len(), 3);
        drop(live);
    }

    #[test]
    fn shelf_writes_on_the_real_schema_only_add_rows() {
        let t = Tmp::new();
        let db = real_db(&t);
        let dump = |db: &Path| -> String {
            let c = Connection::open(db).unwrap();
            let mut out = String::new();
            for q in [
                "SELECT ContentID||ReadStatus||___PercentRead||IFNULL(DateLastRead,'') FROM content ORDER BY ContentID",
                "SELECT BookmarkID||Text||IFNULL(Annotation,'') FROM Bookmark ORDER BY BookmarkID",
            ] {
                let mut st = c.prepare(q).unwrap();
                for r in st.query_map([], |r| r.get::<_, String>(0)).unwrap() {
                    out.push_str(&r.unwrap());
                    out.push('\n');
                }
            }
            out
        };
        let before = dump(&db);
        let ids = vec![
            content_id("Bookx/Sent.kepub.epub"),
            content_id("Sideloaded Book.epub"),
        ];
        assert_eq!(
            add_to_shelf(&db, "Bookx", &ids, "2026-11-12T10:00:00Z").unwrap(),
            2
        );
        assert_eq!(
            dump(&db),
            before,
            "reading state and highlights are untouched"
        );
        let shelves = read_shelves(&db).unwrap();
        assert_eq!(shelves.len(), 1);
        assert_eq!(shelves[0].content_ids.len(), 2);
        // the backup is a complete, readable database with the same rows as before the change
        let bak = t.0.join("KoboReader.sqlite.bookx-backup");
        assert_eq!(dump(&bak), before);
        assert_eq!(db_info(&bak).unwrap().shelves, 0);
        let c = Connection::open(&db).unwrap();
        let (id, internal, ty): (String, String, String) = c
            .query_row(
                "SELECT Id, InternalName, Type FROM Shelf WHERE Name='Bookx'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!((internal.as_str(), ty.as_str()), ("Bookx", "UserTag"));
        let v: i64 = c
            .query_row("SELECT version FROM DbVersion", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 176);
    }

    #[test]
    fn firmware_and_model_come_from_the_version_file() {
        let v = parse_version(
            "N0000000000000,4.9.77,4.46.23852,4.9.77,4.9.77,00000000-0000-0000-0000-000000000390\n",
        )
        .unwrap();
        assert_eq!(v.firmware, "4.46.23852");
        assert_eq!(v.model, Some("Libra Colour"));
        assert_eq!(v.describe(), "Kobo Libra Colour, firmware 4.46.23852");
        assert!(
            !format!("{v:?}").contains("N0000000000000"),
            "the serial number is not kept"
        );
        let other =
            parse_version("N1,3.0,4.1.2,3.0,3.0,00000000-0000-0000-0000-000000000999").unwrap();
        assert_eq!(other.model, None);
        assert!(other
            .describe()
            .contains("model id 00000000-0000-0000-0000-000000000999"));
        assert!(parse_version("").is_none());
        assert!(parse_version("a,b,c").is_none());
        assert!(parse_version("N1,3.0,notaversion,3.0,3.0,id").is_none());
    }
}
