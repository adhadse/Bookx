//! Export of the library as a Calibre library folder (PP-C-18, PP-G-15): `Author/Title (id)/` folders
//! with the book files, `cover.jpg` and `metadata.opf`, and a `metadata.db` in Calibre's schema
//! (user version 27, the structure Calibre 9 creates; see `calibre_schema.sql`). Calibre can open the
//! result, and [`crate::SqliteStore::import_calibre`] reads it back.
//!
//! Nothing is written into the Bookx library. The target folder must be empty (or not exist yet); when
//! the export is cancelled or fails, everything it wrote is removed again. File and folder names are
//! built from sanitised metadata, never from the path of the source file.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bookx_core::library::Book;
use bookx_core::util::{author_sort, sanitize_filename, title_sort, xml_escape};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection};

const SCHEMA: &str = include_str!("calibre_schema.sql");
const SCHEMA_POST: &str = include_str!("calibre_schema_post.sql");
/// `PRAGMA user_version` of the schema above.
pub const CALIBRE_USER_VERSION: i32 = 27;
const NO_DATE: &str = "0101-01-01 00:00:00+00:00";
/// Longest folder or file name part we write (Calibre itself cuts names too).
const MAX_PART: usize = 60;

/// What [`export_calibre`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CalibreExport {
    pub books: usize,
    pub files: usize,
    pub covers: usize,
    pub bytes: u64,
    /// The user cancelled; everything written was removed again.
    pub cancelled: bool,
    /// Files that could not be copied (missing, not a regular file, changed while copying). The
    /// book is still exported with its other files.
    pub problems: Vec<String>,
}

impl CalibreExport {
    pub fn summary(&self) -> String {
        if self.cancelled {
            return "export cancelled, nothing was kept".into();
        }
        let mut s = format!(
            "{} book(s), {} file(s), {} cover(s) exported",
            self.books, self.files, self.covers
        );
        if !self.problems.is_empty() {
            s += &format!(
                "; {} file(s) could not be copied ({})",
                self.problems.len(),
                self.problems
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        s
    }
}

/// Calibre's timestamp format (`2020-01-02 03:04:05.123456+00:00`).
pub(crate) fn calibre_date(d: DateTime<Utc>) -> String {
    d.format("%Y-%m-%d %H:%M:%S%.6f+00:00").to_string()
}

/// `1965`, `1965-08` and `1965-08-01` become a Calibre date; anything else is "no date".
pub(crate) fn calibre_pubdate(published: &str) -> String {
    let mut it = published.trim().split(['-', 'T']).take(3);
    let y: Option<i32> = it.next().and_then(|s| s.trim().parse().ok());
    let m: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let d: u32 = it
        .next()
        .and_then(|s| s.get(..2)?.parse().ok())
        .unwrap_or(1);
    match y.and_then(|y| NaiveDate::from_ymd_opt(y, m, d)) {
        Some(date) if y.is_some_and(|y| y > 101) => format!("{date} 00:00:00+00:00"),
        _ => NO_DATE.into(),
    }
}

/// Calibre stores ISO 639-2 codes (`eng`); Bookx books usually carry the two letter form.
pub fn language_to_calibre(code: &str) -> String {
    const MAP: &[(&str, &str)] = &[
        ("ar", "ara"),
        ("bg", "bul"),
        ("bn", "ben"),
        ("ca", "cat"),
        ("cs", "ces"),
        ("da", "dan"),
        ("de", "deu"),
        ("el", "ell"),
        ("en", "eng"),
        ("es", "spa"),
        ("et", "est"),
        ("fa", "fas"),
        ("fi", "fin"),
        ("fr", "fra"),
        ("he", "heb"),
        ("hi", "hin"),
        ("hr", "hrv"),
        ("hu", "hun"),
        ("id", "ind"),
        ("is", "isl"),
        ("it", "ita"),
        ("ja", "jpn"),
        ("ko", "kor"),
        ("la", "lat"),
        ("lt", "lit"),
        ("lv", "lav"),
        ("nl", "nld"),
        ("no", "nor"),
        ("pl", "pol"),
        ("pt", "por"),
        ("ro", "ron"),
        ("ru", "rus"),
        ("sk", "slk"),
        ("sl", "slv"),
        ("sr", "srp"),
        ("sv", "swe"),
        ("ta", "tam"),
        ("th", "tha"),
        ("tr", "tur"),
        ("uk", "ukr"),
        ("vi", "vie"),
        ("zh", "zho"),
    ];
    let base = code
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    MAP.iter()
        .find(|(a, _)| *a == base)
        .map(|(_, b)| b.to_string())
        .unwrap_or(base)
}

/// A folder or file name part: no separators, no control characters, no leading dots, bounded.
pub(crate) fn name_part(s: &str) -> String {
    let t: String = sanitize_filename(s).chars().take(MAX_PART).collect();
    let t = t.trim().trim_matches('.').trim().to_string();
    if t.is_empty() {
        "Unknown".into()
    } else {
        t
    }
}

fn primary_author(b: &Book) -> String {
    b.meta
        .authors
        .first()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "Unknown".into())
}

/// The book folder relative to the Calibre library: `Author/Title (id)`.
pub(crate) fn book_path(b: &Book) -> String {
    format!(
        "{}/{} ({})",
        name_part(&primary_author(b)),
        name_part(&b.meta.title),
        b.id
    )
}

/// The file name (without extension) Calibre keeps in `data.name`: `Title - Author`.
pub(crate) fn file_stem(b: &Book) -> String {
    format!(
        "{} - {}",
        name_part(&b.meta.title),
        name_part(&primary_author(b))
    )
}

/// `metadata.opf` of one book (OPF 2 with Calibre's `calibre:` meta entries).
fn opf(b: &Book, uuid: &str, has_cover: bool, pubdate: &str) -> String {
    let m = &b.meta;
    let mut s = String::from("<?xml version='1.0' encoding='utf-8'?>\n");
    s += "<package xmlns=\"http://www.idpf.org/2007/opf\" unique-identifier=\"uuid_id\" version=\"2.0\">\n";
    s += "    <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\">\n";
    s += &format!(
        "        <dc:identifier opf:scheme=\"calibre\" id=\"calibre_id\">{}</dc:identifier>\n",
        b.id
    );
    s += &format!(
        "        <dc:identifier opf:scheme=\"uuid\" id=\"uuid_id\">{uuid}</dc:identifier>\n"
    );
    for i in &m.identifiers {
        if !i.scheme.trim().is_empty() && !i.value.trim().is_empty() {
            s += &format!(
                "        <dc:identifier opf:scheme=\"{}\">{}</dc:identifier>\n",
                xml_escape(&i.scheme.to_uppercase()),
                xml_escape(&i.value)
            );
        }
    }
    s += &format!("        <dc:title>{}</dc:title>\n", xml_escape(&m.title));
    for a in &m.authors {
        s += &format!(
            "        <dc:creator opf:file-as=\"{}\" opf:role=\"aut\">{}</dc:creator>\n",
            xml_escape(&author_sort(a)),
            xml_escape(a)
        );
    }
    s += &format!(
        "        <dc:date>{}</dc:date>\n",
        pubdate.replacen(' ', "T", 1)
    );
    if !m.description.is_empty() {
        s += &format!(
            "        <dc:description>{}</dc:description>\n",
            xml_escape(&m.description)
        );
    }
    if !m.publisher.is_empty() {
        s += &format!(
            "        <dc:publisher>{}</dc:publisher>\n",
            xml_escape(&m.publisher)
        );
    }
    if !m.language.is_empty() {
        s += &format!(
            "        <dc:language>{}</dc:language>\n",
            xml_escape(&language_to_calibre(&m.language))
        );
    }
    for t in &m.tags {
        s += &format!("        <dc:subject>{}</dc:subject>\n", xml_escape(t));
    }
    if !m.series.is_empty() {
        s += &format!(
            "        <meta name=\"calibre:series\" content=\"{}\"/>\n        <meta name=\"calibre:series_index\" content=\"{}\"/>\n",
            xml_escape(&m.series),
            bookx_core::opf::format_index(m.series_index)
        );
    }
    if m.rating > 0 {
        s += &format!(
            "        <meta name=\"calibre:rating\" content=\"{}\"/>\n",
            m.rating.min(10)
        );
    }
    s += &format!(
        "        <meta name=\"calibre:timestamp\" content=\"{}\"/>\n        <meta name=\"calibre:title_sort\" content=\"{}\"/>\n",
        calibre_date(b.added).replacen(' ', "T", 1),
        xml_escape(&if m.title_sort.is_empty() {
            title_sort(&m.title)
        } else {
            m.title_sort.clone()
        })
    );
    s += "    </metadata>\n";
    if has_cover {
        s += "    <guide>\n        <reference type=\"cover\" title=\"Cover\" href=\"cover.jpg\"/>\n    </guide>\n";
    }
    s += "</package>\n";
    s
}

/// Removes what the export created. `created_root` says the target folder did not exist before.
fn clean_up(target: &Path, created_root: bool) {
    if created_root {
        let _ = fs::remove_dir_all(target);
        return;
    }
    if let Ok(rd) = fs::read_dir(target) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let _ = fs::remove_dir_all(&p);
            } else {
                let _ = fs::remove_file(&p);
            }
        }
    }
}

/// Checks that `target` can take an export of the library in `root`: it is not inside the library,
/// it does not contain the library, and it is empty or does not exist.
pub fn check_target(root: &Path, target: &Path) -> Result<()> {
    if target.as_os_str().is_empty() {
        bail!("no target folder given");
    }
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (r, t) = (canon(root), {
        // the target may not exist yet: canonicalise the part that does
        let mut existing = target.to_path_buf();
        let mut tail = Vec::new();
        while !existing.exists() {
            match existing.file_name() {
                Some(n) => tail.push(n.to_os_string()),
                None => break,
            }
            if !existing.pop() {
                break;
            }
        }
        let mut c = canon(&existing);
        for n in tail.iter().rev() {
            c.push(n);
        }
        c
    });
    if t.starts_with(&r) || r.starts_with(&t) {
        bail!(
            "{} is inside (or contains) the library folder; choose another folder",
            target.display()
        );
    }
    if target.exists() {
        if !target.is_dir() {
            bail!("{} is not a folder", target.display());
        }
        let mut rd =
            fs::read_dir(target).with_context(|| format!("cannot read {}", target.display()))?;
        if rd.next().is_some() {
            bail!(
                "{} is not empty; export into an empty folder",
                target.display()
            );
        }
    }
    Ok(())
}

/// Copies `src` to `dst` after checking it is a regular file; returns its size.
fn copy_file(src: &Path, dst: &Path) -> Result<u64> {
    let meta = fs::metadata(src).with_context(|| "missing".to_string())?;
    if !meta.is_file() {
        bail!("not a regular file");
    }
    let n = fs::copy(src, dst).with_context(|| "copy failed".to_string())?;
    if n != meta.len() {
        bail!("size changed while copying");
    }
    Ok(n)
}

/// Writes `books` (their files are under `root`) as a Calibre library into `target`.
///
/// `progress` gets `(done, total)` after every book; `cancelled` is asked between books and ends
/// the export with everything removed. Fails before writing anything when `target` is not usable
/// (see [`check_target`]).
pub fn export_calibre(
    books: &[Book],
    root: &Path,
    target: &Path,
    progress: &mut dyn FnMut(usize, usize),
    cancelled: &dyn Fn() -> bool,
) -> Result<CalibreExport> {
    export_calibre_with_columns(books, &[], root, target, progress, cancelled)
}

/// [`export_calibre`] that also writes the custom columns `columns` and every book's values
/// (see `calibre_custom.rs` for what maps to what).
pub fn export_calibre_with_columns(
    books: &[Book],
    columns: &[bookx_core::custom::ColumnDef],
    root: &Path,
    target: &Path,
    progress: &mut dyn FnMut(usize, usize),
    cancelled: &dyn Fn() -> bool,
) -> Result<CalibreExport> {
    check_target(root, target)?;
    let created_root = !target.exists();
    fs::create_dir_all(target).with_context(|| format!("cannot create {}", target.display()))?;
    let r = write_all(books, columns, root, target, progress, cancelled);
    match &r {
        Ok(out) if !out.cancelled => {}
        _ => clean_up(target, created_root),
    }
    r
}

fn write_all(
    books: &[Book],
    columns: &[bookx_core::custom::ColumnDef],
    root: &Path,
    target: &Path,
    progress: &mut dyn FnMut(usize, usize),
    cancelled: &dyn Fn() -> bool,
) -> Result<CalibreExport> {
    let db_path = target.join("metadata.db");
    let mut conn = Connection::open(&db_path).context("cannot create metadata.db")?;
    conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA synchronous = OFF;")?;
    conn.execute_batch(SCHEMA)?;
    let mut out = CalibreExport::default();
    let tx = conn.transaction()?;
    let total = books.len();
    {
        let mut ids = IdMaps::default();
        let mut ins_book = tx.prepare(
            "INSERT INTO books(id, title, sort, timestamp, pubdate, series_index, author_sort, path,
                               uuid, has_cover, last_modified)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for (n, b) in books.iter().enumerate() {
            if cancelled() {
                out.cancelled = true;
                break;
            }
            progress(n, total);
            let m = &b.meta;
            let rel = book_path(b);
            let dir = target.join(&rel);
            fs::create_dir_all(&dir)?;
            let uuid = uuid::Uuid::new_v4().to_string();
            let stem = file_stem(b);

            // files
            let mut seen_formats: Vec<String> = Vec::new();
            for f in b.all_formats() {
                let ext = Path::new(&f.file)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let ext: String = ext.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                if ext.is_empty() || ext.len() > 8 || seen_formats.contains(&ext) {
                    continue;
                }
                let src: PathBuf = root.join(&f.file);
                let dst = dir.join(format!("{stem}.{ext}"));
                match copy_file(&src, &dst) {
                    Ok(size) => {
                        out.files += 1;
                        out.bytes += size;
                        seen_formats.push(ext.clone());
                        tx.execute(
                            "INSERT INTO data(book, format, uncompressed_size, name) VALUES(?1, ?2, ?3, ?4)",
                            params![b.id as i64, ext.to_uppercase(), size as i64, stem],
                        )?;
                    }
                    Err(e) => out.problems.push(format!("{}: {e:#}", m.title)),
                }
            }
            // cover
            let has_cover = match b.cover.as_ref().map(|c| root.join(c)) {
                Some(src) => match copy_file(&src, &dir.join("cover.jpg")) {
                    Ok(size) => {
                        out.covers += 1;
                        out.bytes += size;
                        true
                    }
                    Err(_) => false,
                },
                None => false,
            };
            let pubdate = calibre_pubdate(&m.published);
            fs::write(dir.join("metadata.opf"), opf(b, &uuid, has_cover, &pubdate))?;

            // catalogue rows
            let authors: Vec<String> = m
                .authors
                .iter()
                .map(|a| a.trim().replace(',', "|"))
                .filter(|a| !a.is_empty())
                .collect();
            let book_author_sort = if m.author_sort.is_empty() {
                authors
                    .iter()
                    .map(|a| author_sort(&a.replace('|', ",")))
                    .collect::<Vec<_>>()
                    .join(" & ")
            } else {
                m.author_sort.clone()
            };
            let sort = if m.title_sort.is_empty() {
                title_sort(&m.title)
            } else {
                m.title_sort.clone()
            };
            ins_book.execute(params![
                b.id as i64,
                m.title,
                sort,
                calibre_date(b.added),
                pubdate,
                if m.series.is_empty() {
                    1.0
                } else {
                    m.series_index
                },
                book_author_sort,
                rel,
                uuid,
                has_cover as i64,
                calibre_date(b.modified),
            ])?;
            for a in &authors {
                let sort = if authors.len() == 1 && !m.author_sort.is_empty() {
                    m.author_sort.clone()
                } else {
                    author_sort(&a.replace('|', ","))
                };
                let aid = ids.get(&tx, "authors", a, Some(&sort))?;
                tx.execute(
                    "INSERT OR IGNORE INTO books_authors_link(book, author) VALUES(?1, ?2)",
                    params![b.id as i64, aid],
                )?;
            }
            let mut tags_seen: Vec<String> = Vec::new();
            for t in &m.tags {
                let t = t.trim().replace(',', ";");
                if t.is_empty() || tags_seen.contains(&t.to_lowercase()) {
                    continue;
                }
                let tid = ids.get(&tx, "tags", &t, None)?;
                tx.execute(
                    "INSERT INTO books_tags_link(book, tag) VALUES(?1, ?2)",
                    params![b.id as i64, tid],
                )?;
                tags_seen.push(t.to_lowercase());
            }
            if !m.series.trim().is_empty() {
                let sid = ids.get(&tx, "series", m.series.trim(), Some(&title_sort(&m.series)))?;
                tx.execute(
                    "INSERT INTO books_series_link(book, series) VALUES(?1, ?2)",
                    params![b.id as i64, sid],
                )?;
            }
            if !m.publisher.trim().is_empty() {
                let pid = ids.get(
                    &tx,
                    "publishers",
                    m.publisher.trim(),
                    Some(m.publisher.trim()),
                )?;
                tx.execute(
                    "INSERT INTO books_publishers_link(book, publisher) VALUES(?1, ?2)",
                    params![b.id as i64, pid],
                )?;
            }
            if !m.description.is_empty() {
                tx.execute(
                    "INSERT INTO comments(book, text) VALUES(?1, ?2)",
                    params![b.id as i64, m.description],
                )?;
            }
            if m.rating > 0 {
                let rid = ids.rating(&tx, m.rating.min(10) as i64)?;
                tx.execute(
                    "INSERT INTO books_ratings_link(book, rating) VALUES(?1, ?2)",
                    params![b.id as i64, rid],
                )?;
            }
            let lang = language_to_calibre(&m.language);
            if !lang.is_empty() {
                let lid = ids.get(&tx, "languages", &lang, None)?;
                tx.execute(
                    "INSERT INTO books_languages_link(book, lang_code, item_order) VALUES(?1, ?2, 0)",
                    params![b.id as i64, lid],
                )?;
            }
            for i in &m.identifiers {
                let (k, v) = (i.scheme.trim().to_lowercase(), i.value.trim());
                if k.is_empty() || v.is_empty() {
                    continue;
                }
                tx.execute(
                    "INSERT OR IGNORE INTO identifiers(book, type, val) VALUES(?1, ?2, ?3)",
                    params![b.id as i64, k, v],
                )?;
            }
            tx.execute(
                "INSERT INTO books_pages_link(book, needs_scan) VALUES(?1, 1)",
                params![b.id as i64],
            )?;
            out.books += 1;
        }
        if !out.cancelled {
            crate::calibre_custom::write_calibre_custom(&tx, columns, books)?;
            tx.execute(
                "INSERT INTO library_id(uuid) VALUES(?1)",
                params![uuid::Uuid::new_v4().to_string()],
            )?;
            for (k, v) in [
                ("bools_are_tristate", "true"),
                ("user_categories", "{}"),
                ("saved_searches", "{}"),
                ("grouped_search_terms", "{}"),
            ] {
                tx.execute(
                    "INSERT INTO preferences(key, val) VALUES(?1, ?2)",
                    params![k, v],
                )?;
            }
        }
    }
    if out.cancelled {
        drop(tx);
        drop(conn);
        return Ok(out);
    }
    tx.commit()?;
    conn.execute_batch(SCHEMA_POST)?;
    conn.execute_batch(&format!("PRAGMA user_version = {CALIBRE_USER_VERSION};"))?;
    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        bail!("the exported metadata.db failed its integrity check: {check}");
    }
    progress(total, total);
    Ok(out)
}

/// Ids of rows in Calibre's lookup tables, by (table, lower-cased name).
#[derive(Default)]
struct IdMaps {
    names: HashMap<(&'static str, String), i64>,
    ratings: HashMap<i64, i64>,
}

impl IdMaps {
    fn get(
        &mut self,
        tx: &rusqlite::Transaction,
        table: &'static str,
        name: &str,
        sort: Option<&str>,
    ) -> Result<i64> {
        let key = (table, name.to_lowercase());
        if let Some(id) = self.names.get(&key) {
            return Ok(*id);
        }
        match table {
            "authors" => tx.execute(
                "INSERT INTO authors(name, sort) VALUES(?1, ?2)",
                params![name, sort.unwrap_or(name)],
            )?,
            "series" => tx.execute(
                "INSERT INTO series(name, sort) VALUES(?1, ?2)",
                params![name, sort.unwrap_or(name)],
            )?,
            "publishers" => tx.execute(
                "INSERT INTO publishers(name, sort) VALUES(?1, ?2)",
                params![name, sort.unwrap_or(name)],
            )?,
            "tags" => tx.execute("INSERT INTO tags(name) VALUES(?1)", params![name])?,
            "languages" => {
                tx.execute("INSERT INTO languages(lang_code) VALUES(?1)", params![name])?
            }
            _ => bail!("unknown lookup table {table}"),
        };
        let id = tx.last_insert_rowid();
        self.names.insert(key, id);
        Ok(id)
    }

    fn rating(&mut self, tx: &rusqlite::Transaction, r: i64) -> Result<i64> {
        if let Some(id) = self.ratings.get(&r) {
            return Ok(*id);
        }
        tx.execute("INSERT INTO ratings(rating) VALUES(?1)", params![r])?;
        let id = tx.last_insert_rowid();
        self.ratings.insert(r, id);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_in_calibre_format() {
        assert_eq!(calibre_pubdate("1965-08-01"), "1965-08-01 00:00:00+00:00");
        assert_eq!(calibre_pubdate("1965"), "1965-01-01 00:00:00+00:00");
        assert_eq!(calibre_pubdate("1965-08"), "1965-08-01 00:00:00+00:00");
        assert_eq!(
            calibre_pubdate("2001-02-03T04:05:06Z"),
            "2001-02-03 00:00:00+00:00"
        );
        assert_eq!(calibre_pubdate(""), NO_DATE);
        assert_eq!(calibre_pubdate("garbage"), NO_DATE);
        assert_eq!(calibre_pubdate("0101-01-01"), NO_DATE);
        assert_eq!(calibre_pubdate("1999-13-40"), NO_DATE);
    }

    #[test]
    fn languages_become_three_letter_codes() {
        assert_eq!(language_to_calibre("en"), "eng");
        assert_eq!(language_to_calibre("EN-us"), "eng");
        assert_eq!(language_to_calibre("de_AT"), "deu");
        assert_eq!(language_to_calibre("eng"), "eng");
        assert_eq!(language_to_calibre(""), "");
    }

    #[test]
    fn names_cannot_escape_the_folder() {
        for bad in ["../../etc/passwd", "..", ".", "a/b\\c", "/abs", "x\0y", ""] {
            let p = name_part(bad);
            assert!(
                !p.contains('/') && !p.contains('\\') && !p.contains('\0'),
                "{p}"
            );
            assert!(!p.starts_with('.') && !p.is_empty(), "{p}");
        }
        assert!(name_part(&"é".repeat(500)).chars().count() <= MAX_PART);
    }

    #[test]
    fn book_folder_follows_calibre_layout() {
        let mut b = Book {
            id: 7,
            file: "x.epub".into(),
            cover: None,
            meta: Default::default(),
            added: Utc::now(),
            modified: Utc::now(),
            last_read: None,
            size: 0,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: vec![],
            resources: vec![],
            drm: String::new(),
            custom: Default::default(),
            marked: false,
        };
        b.meta.title = "../Dune: Part 1".into();
        b.meta.authors = vec!["Frank/Herbert".into()];
        assert_eq!(book_path(&b), "Frank_Herbert/_Dune_ Part 1 (7)");
        assert_eq!(file_stem(&b), "_Dune_ Part 1 - Frank_Herbert");
        b.meta.authors.clear();
        assert!(book_path(&b).starts_with("Unknown/"));
    }

    #[test]
    fn opf_escapes_and_has_series() {
        let mut b = Book {
            id: 1,
            file: "x.epub".into(),
            cover: None,
            meta: Default::default(),
            added: Utc::now(),
            modified: Utc::now(),
            last_read: None,
            size: 0,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: vec![],
            resources: vec![],
            drm: String::new(),
            custom: Default::default(),
            marked: false,
        };
        b.meta.title = "A & B <c>".into();
        b.meta.authors = vec!["Ada Lovelace".into()];
        b.meta.series = "S".into();
        b.meta.series_index = 2.0;
        let x = opf(&b, "u", true, NO_DATE);
        assert!(x.contains("A &amp; B &lt;c&gt;"));
        assert!(x.contains("opf:file-as=\"Lovelace, Ada\""));
        assert!(x.contains("calibre:series\" content=\"S\""));
        assert!(x.contains("href=\"cover.jpg\""));
        assert!(!opf(&b, "u", false, NO_DATE).contains("<guide>"));
    }
}
