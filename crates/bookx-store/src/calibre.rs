//! Import of a Calibre library (`metadata.db`). Files are not copied: the produced books point at
//! `<book path>/<name>.epub` inside the Calibre folder.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use bookx_core::library::Book;
use bookx_core::opf::{Identifier, Metadata};
use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{Connection, OpenFlags};

use crate::store::{SqliteStore, all_books_conn, alloc_id, put_row};

/// What [`SqliteStore::import_calibre`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: usize,
    /// Calibre books without an EPUB file.
    pub skipped_no_epub: usize,
    /// Books already in the store (same title and first author, ignoring case).
    pub duplicates: usize,
}

/// Parses Calibre's timestamps (`2020-01-02 03:04:05.123456+00:00`).
pub(crate) fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(d) = DateTime::parse_from_rfc3339(&s.replacen(' ', "T", 1)) {
        return Some(d.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
        .ok()
        .map(|n| n.and_utc())
}

/// Calibre marks "no publication date" with year 0101 (or 0000).
pub(crate) fn published_string(s: &str) -> String {
    match parse_date(s) {
        Some(d) if chrono::Datelike::year(&d) > 101 => d.format("%Y-%m-%d").to_string(),
        _ => String::new(),
    }
}

/// Calibre stores commas inside author names as `|`.
pub(crate) fn author_name(s: &str) -> String {
    s.replace('|', ",")
}

fn dup_key(title: &str, first_author: Option<&str>) -> (String, String) {
    (
        title.to_lowercase(),
        first_author.map(str::to_lowercase).unwrap_or_default(),
    )
}

fn strings(conn: &Connection, sql: &str, id: i64) -> Result<Vec<String>> {
    let mut st = conn.prepare_cached(sql)?;
    let rows = st.query_map([id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

struct Row {
    id: i64,
    title: String,
    sort: Option<String>,
    timestamp: Option<String>,
    last_modified: Option<String>,
    pubdate: Option<String>,
    series_index: Option<f64>,
    author_sort: Option<String>,
    path: String,
    has_cover: bool,
}

/// The full result of [`SqliteStore::import_calibre_with`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CalibreImport {
    pub report: ImportReport,
    /// Ids of the books that were added.
    pub ids: Vec<u64>,
    /// Titles of Calibre books that were left out because they have no EPUB.
    pub skipped: Vec<String>,
    /// Titles that were already in the library.
    pub duplicate_titles: Vec<String>,
    /// Books whose files could not be copied into the library (they stay linked to Calibre).
    pub copy_failed: Vec<String>,
    /// Custom columns that were added to the library.
    pub custom_columns: Vec<String>,
    /// Calibre custom columns that were left out, with the reason.
    pub skipped_columns: Vec<String>,
}

/// Which part of [`import_calibre_into`] is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Reading Calibre's `metadata.db`.
    Reading,
    /// Copying book files into the library.
    Copying,
}

impl CalibreImport {
    /// A few lines for the user (and the command line).
    pub fn summary(&self) -> String {
        let mut s = format!("{} book(s) imported", self.report.imported);
        if !self.duplicate_titles.is_empty() {
            s += &format!(
                "; {} already in the library ({})",
                self.duplicate_titles.len(),
                preview(&self.duplicate_titles)
            );
        }
        if !self.skipped.is_empty() {
            s += &format!(
                "; {} skipped because they have no EPUB ({})",
                self.skipped.len(),
                preview(&self.skipped)
            );
        }
        if !self.custom_columns.is_empty() {
            s += &format!(
                "; {} custom column(s) added ({})",
                self.custom_columns.len(),
                preview(&self.custom_columns)
            );
        }
        if !self.skipped_columns.is_empty() {
            s += &format!(
                "; {} custom column(s) left out ({})",
                self.skipped_columns.len(),
                preview(&self.skipped_columns)
            );
        }
        if !self.copy_failed.is_empty() {
            s += &format!(
                "; {} could not be copied and stay in the Calibre folder ({})",
                self.copy_failed.len(),
                preview(&self.copy_failed)
            );
        }
        s
    }
}

fn preview(titles: &[String]) -> String {
    let mut shown: Vec<&str> = titles.iter().take(3).map(String::as_str).collect();
    if titles.len() > 3 {
        shown.push("…");
    }
    shown.join(", ")
}

/// Imports a Calibre library into the Bookx library in `root`.
///
/// The books are first linked to their files in the Calibre folder. With `copy` they are then
/// copied into `root` following the library's layout, so the library stands on its own; a book that
/// cannot be copied stays linked and is listed in `copy_failed`. The caller reloads its [`Library`]
/// afterwards. `progress` gets the stage and `(done, total)`.
pub fn import_calibre_into(
    store: &std::sync::Arc<SqliteStore>,
    root: &Path,
    calibre_dir: &Path,
    copy: bool,
    progress: &mut dyn FnMut(Stage, usize, usize),
) -> Result<CalibreImport> {
    let mut out = store.import_calibre_with(calibre_dir, true, &mut |d, n| {
        progress(Stage::Reading, d, n)
    })?;
    if copy && !out.ids.is_empty() {
        let mut lib = bookx_core::library::Library::with_backend(
            root,
            Box::new(crate::catalog::SqliteCatalog::new(store.clone())),
        )?;
        let total = out.ids.len();
        for (n, id) in out.ids.clone().into_iter().enumerate() {
            progress(Stage::Copying, n, total);
            if let Err(e) = lib.copy_into_library(id) {
                let title = lib
                    .get(id)
                    .map(|b| b.meta.title.clone())
                    .unwrap_or_default();
                out.copy_failed.push(format!("{title}: {e:#}"));
            }
        }
        lib.save()?;
        progress(Stage::Copying, total, total);
    }
    Ok(out)
}

/// A path from `metadata.db` that stays inside the library: relative, no `..`, no backslash, no NUL.
fn safe_rel(p: &str) -> bool {
    let drive = p.len() >= 2 && p.as_bytes()[1] == b':';
    !(p.is_empty()
        || p.starts_with('/')
        || p.contains(['\\', '\0'])
        || p.split('/').any(|c| c == ".." || c.is_empty())
        || drive)
}

impl SqliteStore {
    /// Reads `<library_dir>/metadata.db` and adds every EPUB book with a fresh id. The book paths
    /// are relative to `library_dir`.
    pub fn import_calibre(&self, library_dir: &Path) -> Result<ImportReport> {
        Ok(self
            .import_calibre_with(library_dir, false, &mut |_, _| {})?
            .report)
    }

    /// Like [`import_calibre`](Self::import_calibre) with a progress callback `(done, total)`
    /// and the full result. With `absolute` the book and cover paths are absolute, so the books
    /// stay usable from any Bookx library folder (the Calibre files are linked, not copied).
    /// Nothing is added when this fails.
    pub fn import_calibre_with(
        &self,
        library_dir: &Path,
        absolute: bool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<CalibreImport> {
        let library_dir = if absolute {
            library_dir
                .canonicalize()
                .unwrap_or_else(|_| library_dir.to_path_buf())
        } else {
            library_dir.to_path_buf()
        };
        let library_dir = library_dir.as_path();
        let db = library_dir.join("metadata.db");
        if !db.is_file() {
            bail!("{} has no metadata.db", library_dir.display());
        }
        let cal = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("cannot open {}", db.display()))?;
        let rows = {
            let mut st = cal
                .prepare(
                    "SELECT id, title, sort, timestamp, last_modified, pubdate, series_index,
                            author_sort, path, has_cover FROM books ORDER BY id",
                )
                .context("not a Calibre metadata.db")?;
            let rows = st.query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    sort: r.get(2)?,
                    timestamp: r.get(3)?,
                    last_modified: r.get(4)?,
                    pubdate: r.get(5)?,
                    series_index: r.get(6)?,
                    author_sort: r.get(7)?,
                    path: r.get(8)?,
                    has_cover: r.get::<_, Option<i64>>(9)?.unwrap_or(0) != 0,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut custom = crate::calibre_custom::read_calibre_custom(&cal);
        let mut out = CalibreImport {
            skipped_columns: std::mem::take(&mut custom.skipped),
            ..Default::default()
        };
        let report = &mut out.report;
        let total = rows.len();
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // the columns of the Calibre library join the ones already defined; one with the same label
        // but another type is left alone and its values are not imported
        let mut have = crate::custom::load_columns(&tx);
        let mut usable: HashSet<String> = HashSet::new();
        let mut added_cols = false;
        for d in &custom.defs {
            match have.iter().find(|h| h.label == d.label) {
                Some(h) if h.kind == d.kind => {
                    usable.insert(d.label.clone());
                }
                Some(_) => out.skipped_columns.push(format!(
                    "{} (a column with that label and another type exists)",
                    d.name
                )),
                None if have.len() < bookx_core::custom::MAX_COLUMNS
                    && !have.iter().any(|h| h.name.eq_ignore_ascii_case(&d.name)) =>
                {
                    have.push(d.clone());
                    usable.insert(d.label.clone());
                    out.custom_columns.push(d.name.clone());
                    added_cols = true;
                }
                None => out
                    .skipped_columns
                    .push(format!("{} (name already used)", d.name)),
            }
        }
        if added_cols {
            crate::custom::store_columns(&tx, &have)?;
        }
        let mut seen: HashSet<(String, String)> = all_books_conn(&tx)?
            .iter()
            .map(|b| dup_key(&b.meta.title, b.meta.authors.first().map(String::as_str)))
            .collect();

        for (n, row) in rows.into_iter().enumerate() {
            progress(n, total);
            let epub: Option<(String, Option<i64>)> = {
                let mut st = cal.prepare_cached(
                    "SELECT name, uncompressed_size FROM data
                     WHERE book = ?1 AND upper(format) = 'EPUB' ORDER BY id LIMIT 1",
                )?;
                let mut it = st.query_map([row.id], |r| Ok((r.get(0)?, r.get(1)?)))?;
                it.next().transpose()?
            };
            let Some((name, size)) = epub else {
                out.skipped.push(row.title.clone());
                report.skipped_no_epub += 1;
                continue;
            };
            // a hostile metadata.db could point at files outside the library: only plain relative names are followed
            if !safe_rel(&row.path) || !safe_rel(&name) {
                out.skipped.push(row.title.clone());
                report.skipped_no_epub += 1;
                continue;
            }
            let mut authors: Vec<String> = strings(
                &cal,
                "SELECT a.name FROM authors a JOIN books_authors_link l ON l.author = a.id
                 WHERE l.book = ?1 ORDER BY l.id",
                row.id,
            )?
            .iter()
            .map(|a| author_name(a))
            .collect();
            if authors.is_empty() {
                authors.push("Unknown".into());
            }
            let key = dup_key(&row.title, authors.first().map(String::as_str));
            if !seen.insert(key) {
                out.duplicate_titles.push(row.title.clone());
                report.duplicates += 1;
                continue;
            }
            let tags = strings(
                &cal,
                "SELECT t.name FROM tags t JOIN books_tags_link l ON l.tag = t.id
                 WHERE l.book = ?1 ORDER BY t.name",
                row.id,
            )?;
            let series = strings(
                &cal,
                "SELECT s.name FROM series s JOIN books_series_link l ON l.series = s.id
                 WHERE l.book = ?1",
                row.id,
            )?
            .into_iter()
            .next()
            .unwrap_or_default();
            let publisher = strings(
                &cal,
                "SELECT p.name FROM publishers p JOIN books_publishers_link l ON l.publisher = p.id
                 WHERE l.book = ?1",
                row.id,
            )?
            .into_iter()
            .next()
            .unwrap_or_default();
            let description = strings(&cal, "SELECT text FROM comments WHERE book = ?1", row.id)?
                .into_iter()
                .next()
                .unwrap_or_default();
            let language = strings(
                &cal,
                "SELECT g.lang_code FROM languages g JOIN books_languages_link l ON l.lang_code = g.id
                 WHERE l.book = ?1 ORDER BY l.item_order",
                row.id,
            )?
            .into_iter()
            .next()
            .unwrap_or_default();
            let rating: i64 = cal
                .prepare_cached(
                    "SELECT r.rating FROM ratings r JOIN books_ratings_link l ON l.rating = r.id
                     WHERE l.book = ?1",
                )?
                .query_map([row.id], |r| r.get::<_, Option<i64>>(0))?
                .next()
                .transpose()?
                .flatten()
                .unwrap_or(0);
            let identifiers = {
                let mut st = cal.prepare_cached(
                    "SELECT type, val FROM identifiers WHERE book = ?1 ORDER BY id",
                )?;
                let rows = st.query_map([row.id], |r| {
                    Ok(Identifier {
                        scheme: r.get(0)?,
                        value: r.get(1)?,
                        xml_id: None,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };

            let added = row
                .timestamp
                .as_deref()
                .and_then(parse_date)
                .unwrap_or_else(Utc::now);
            let modified = row
                .last_modified
                .as_deref()
                .and_then(parse_date)
                .unwrap_or(added);
            let meta = Metadata {
                title_sort: row
                    .sort
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| row.title.clone()),
                title: row.title,
                author_sort: row
                    .author_sort
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| authors.join(" & ")),
                authors,
                publisher,
                language,
                description,
                tags,
                series_index: if series.is_empty() {
                    0.0
                } else {
                    row.series_index.unwrap_or(1.0)
                },
                series,
                identifiers,
                published: row
                    .pubdate
                    .as_deref()
                    .map(published_string)
                    .unwrap_or_default(),
                rights: String::new(),
                rating: rating.clamp(0, 10) as u8,
            };
            let prefix = if absolute {
                library_dir.join(&row.path).to_string_lossy().to_string()
            } else {
                row.path.clone()
            };
            let book = Book {
                id: alloc_id(&tx)?,
                file: format!("{prefix}/{name}.epub"),
                cover: row.has_cover.then(|| format!("{prefix}/cover.jpg")),
                meta,
                added,
                modified,
                last_read: None,
                size: size.unwrap_or(0).max(0) as u64,
                progress: 0.0,
                position: None,
                bookmarks: Vec::new(),
                highlights: Vec::new(),
                formats: Vec::new(),
                resources: Vec::new(),
                drm: String::new(),
                custom: custom
                    .values
                    .remove(&row.id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(l, _)| usable.contains(l))
                    .collect(),
                marked: false,
                source_ids: Vec::new(),
            };
            put_row(&tx, &book)?;
            out.ids.push(book.id);
            report.imported += 1;
        }
        tx.commit()?;
        progress(total, total);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_calibre_dates() {
        let d = parse_date("2020-01-02 03:04:05.123456+00:00").unwrap();
        assert_eq!(d.to_rfc3339(), "2020-01-02T03:04:05.123456+00:00");
        let d = parse_date("2020-01-02 03:04:05+02:00").unwrap();
        assert_eq!(d.to_rfc3339(), "2020-01-02T01:04:05+00:00");
        let d = parse_date("2020-01-02 03:04:05").unwrap();
        assert_eq!(d.timestamp(), 1577934245);
        assert!(parse_date("yesterday").is_none());
        assert!(parse_date("").is_none());
    }

    #[test]
    fn unset_pubdate_is_empty() {
        assert_eq!(published_string("0101-01-01 00:00:00+00:00"), "");
        assert_eq!(published_string("garbage"), "");
        assert_eq!(published_string("1954-07-29 00:00:00+00:00"), "1954-07-29");
    }

    #[test]
    fn author_pipes_become_commas() {
        assert_eq!(author_name("Doe| John"), "Doe, John");
        assert_eq!(author_name("Plain"), "Plain");
    }

    #[test]
    fn duplicate_key_ignores_case() {
        assert_eq!(
            dup_key("Dune", Some("Frank")),
            dup_key("DUNE", Some("frank"))
        );
        assert_ne!(dup_key("Dune", Some("A")), dup_key("Dune", None));
    }
}
