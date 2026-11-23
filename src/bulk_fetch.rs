//! Fetching metadata for many books at once (roadmap PP-G-1): the lookups run in the background,
//! every difference becomes a *row* of a review table (book, field, current → proposed, source,
//! confidence) and only the rows the user accepts are written. Each applied field records where it
//! came from, and every book is one entry of the operation log, so the whole batch can be undone.
//!
//! No GTK in here, so it is unit tested; the table is `ui/bulkfetch.rs`.

use crate::i18n::tr;
use crate::i18n::trf;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use bookx_sources::cover_quality as cq;
use bookx_sources::fetch::Fetcher;
use bookx_store::SqliteStore;

use crate::core::library::Library;
use crate::core::opf::{Metadata, format_index};
use crate::metadata_fetch::{self as mf, Fetched, Merge};

/// The fields a lookup can propose (`cover` is handled apart from the OPF metadata).
pub const FIELDS: [&str; 10] = [
    "title",
    "authors",
    "series",
    "publisher",
    "published",
    "language",
    "tags",
    "identifiers",
    "description",
    "cover",
];

/// The settings of this feature (in the config file).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MetadataPrefs {
    /// After a bulk lookup, write the values that only fill an empty field without asking.
    pub auto_apply_safe: bool,
    /// Run the check before converting or e-mailing a book.
    pub preflight: bool,
    /// Write a report next to every converted file.
    pub conversion_report: bool,
    /// After an import, look the new books up and fill in the fields they have nothing in.
    pub fetch_on_import: bool,
}

impl Default for MetadataPrefs {
    fn default() -> Self {
        MetadataPrefs {
            auto_apply_safe: false,
            preflight: false,
            conversion_report: true,
            fetch_on_import: false,
        }
    }
}

/// What the lookup of one book came back with.
#[derive(Debug, Clone)]
pub struct BookResult {
    pub book: u64,
    pub title: String,
    pub current: Metadata,
    pub has_cover: bool,
    /// Size of the cover the book has now, when it is known (see `bookx_sources::cover_quality`).
    pub cover_px: Option<(u32, u32)>,
    pub found: Option<Fetched>,
    pub error: Option<String>,
}

/// One line of the review table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub book: u64,
    pub book_title: String,
    pub field: &'static str,
    pub current: String,
    pub proposed: String,
    pub source: String,
    pub confidence: f64,
    /// The book has nothing in this field, so taking the value loses nothing.
    pub safe: bool,
    pub accepted: bool,
}

/// How a field of `m` is shown in the table.
pub fn show_field(m: &Metadata, field: &str) -> String {
    let short = |s: &str| {
        let plain: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if plain.chars().count() > 90 {
            format!("{}…", plain.chars().take(90).collect::<String>())
        } else {
            plain
        }
    };
    match field {
        "title" => m.title.clone(),
        "authors" => m.authors.join(" & "),
        "series" => {
            if m.series.trim().is_empty() {
                String::new()
            } else {
                format!("{} #{}", m.series, format_index(m.series_index))
            }
        }
        "publisher" => m.publisher.clone(),
        "published" => m.published.clone(),
        "language" => m.language.clone(),
        "tags" => m.tags.join(", "),
        "identifiers" => m
            .identifiers
            .iter()
            .map(|i| format!("{}:{}", i.scheme, i.value))
            .collect::<Vec<_>>()
            .join(", "),
        "description" => short(&crate::core::html::parse_html(&m.description, "d.html").to_text()),
        _ => String::new(),
    }
}

/// What the review table shows for the cover now, and whether the cover is missing (so a new one
/// is safe to take).
pub fn cover_row(has_cover: bool, current: Option<(u32, u32)>) -> (String, bool) {
    if !has_cover {
        return (String::new(), true);
    }
    match current {
        Some(s) => (format!("has a cover ({})", cq::describe_quality(s)), false),
        None => ("has a cover".into(), false),
    }
}

/// The proposals for one book. A field is *safe* when the book has nothing there yet.
pub fn rows_for(r: &BookResult) -> Vec<Row> {
    let Some(f) = &r.found else {
        return Vec::new();
    };
    let full = mf::apply(&r.current, &f.meta, Merge::Replace);
    let fill = mf::apply(&r.current, &f.meta, Merge::FillEmpty);
    let safe: Vec<&str> = mf::changed_fields(&r.current, &fill);
    let all_sources = f.sources.join(", ");
    let source_of = |field: &str| -> String {
        f.field_sources
            .get(field)
            .cloned()
            .unwrap_or_else(|| all_sources.clone())
    };
    let mut rows: Vec<Row> = mf::changed_fields(&r.current, &full)
        .into_iter()
        .map(|field| Row {
            book: r.book,
            book_title: r.title.clone(),
            field,
            current: show_field(&r.current, field),
            proposed: show_field(&full, field),
            source: source_of(field),
            confidence: f.confidence,
            safe: safe.contains(&field),
            accepted: false,
        })
        .collect();
    if let Some(c) = &f.cover {
        let candidate = cq::size_of(c);
        let (current, safe) = cover_row(r.has_cover, r.cover_px);
        let proposed = match candidate {
            Some(s) => format!(
                "new cover ({}, {} KB)",
                cq::describe(s),
                c.len().div_ceil(1024)
            ),
            None => format!("new cover ({} KB)", c.len().div_ceil(1024)),
        };
        // a clearly better cover also replaces a poor one without asking twice
        let upgrade = r.has_cover
            && matches!((r.cover_px, candidate), (Some(cur), Some(new)) if cq::grade(cur) < cq::Grade::Good && cq::is_upgrade(cur, new));
        rows.push(Row {
            book: r.book,
            book_title: r.title.clone(),
            field: "cover",
            current,
            proposed,
            source: source_of("cover"),
            confidence: f.confidence,
            safe: safe || upgrade,
            accepted: false,
        });
    }
    rows.sort_by_key(|row| {
        (
            row.book,
            FIELDS.iter().position(|f| *f == row.field).unwrap_or(99),
        )
    });
    rows
}

/// `current` with only the named `fields` of `found` taken (not `cover`).
pub fn merge_fields(current: &Metadata, found: &Metadata, fields: &[&str]) -> Metadata {
    let full = mf::apply(current, found, Merge::Replace);
    let mut m = current.clone();
    for f in fields {
        match *f {
            "title" => {
                m.title = full.title.clone();
                m.title_sort = full.title_sort.clone();
            }
            "authors" => {
                m.authors = full.authors.clone();
                m.author_sort = full.author_sort.clone();
            }
            "series" => {
                m.series = full.series.clone();
                m.series_index = full.series_index;
            }
            "publisher" => m.publisher = full.publisher.clone(),
            "published" => m.published = full.published.clone(),
            "language" => m.language = full.language.clone(),
            "tags" => m.tags = full.tags.clone(),
            "identifiers" => m.identifiers = full.identifiers.clone(),
            "description" => m.description = full.description.clone(),
            _ => {}
        }
    }
    m
}

/// All lookups of a batch and the table made from them.
#[derive(Debug, Clone, Default)]
pub struct Review {
    pub results: Vec<BookResult>,
    pub rows: Vec<Row>,
}

impl Review {
    pub fn new(results: Vec<BookResult>) -> Review {
        let rows = results.iter().flat_map(rows_for).collect();
        Review { results, rows }
    }

    pub fn accept_all(&mut self, on: bool) {
        self.rows.iter_mut().for_each(|r| r.accepted = on);
    }

    /// Accept (or not) every row of one field, over all books.
    pub fn accept_field(&mut self, field: &str, on: bool) {
        self.rows
            .iter_mut()
            .filter(|r| r.field == field)
            .for_each(|r| r.accepted = on);
    }

    /// Accepts the rows that only fill an empty field, and nothing else.
    pub fn accept_safe(&mut self) {
        self.rows.iter_mut().for_each(|r| r.accepted = r.safe);
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(r) = self.rows.get_mut(index) {
            r.accepted = !r.accepted;
        }
    }

    pub fn accepted(&self) -> usize {
        self.rows.iter().filter(|r| r.accepted).count()
    }

    /// Books whose lookup failed, with the reason.
    pub fn failures(&self) -> Vec<(&str, &str)> {
        self.results
            .iter()
            .filter_map(|r| r.error.as_deref().map(|e| (r.title.as_str(), e)))
            .collect()
    }

    /// Books the sources knew nothing about.
    pub fn not_found(&self) -> usize {
        self.results
            .iter()
            .filter(|r| r.error.is_none() && r.found.is_none())
            .count()
    }

    /// The fields that occur in the table, in table order (for the *by field* buttons).
    pub fn fields_present(&self) -> Vec<&'static str> {
        let set: BTreeSet<&str> = self.rows.iter().map(|r| r.field).collect();
        FIELDS.iter().copied().filter(|f| set.contains(f)).collect()
    }

    /// Takes the accepted rows out of the table (they are about to be applied).
    pub fn take_accepted(&mut self) -> Vec<Row> {
        let (done, rest): (Vec<Row>, Vec<Row>) = self.rows.drain(..).partition(|r| r.accepted);
        self.rows = rest;
        done
    }

    /// One line for the header: how the lookups went.
    pub fn summary(&self) -> String {
        let n = self.results.len();
        let found = self.results.iter().filter(|r| r.found.is_some()).count();
        let mut s = format!("{found} of {n} books found, {} proposals", self.rows.len());
        let failed = self.failures().len();
        if failed > 0 {
            s += &format!(", {failed} failed");
        }
        s
    }
}

/// What applying rows did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub books: usize,
    pub fields: usize,
    /// Entries added to the operation log (the number to undo).
    pub ops: usize,
    /// Book → the fields written, for the provenance to be taken back on undo.
    pub touched: Vec<(u64, Vec<String>)>,
    pub errors: Vec<String>,
}

fn image_ext(data: &[u8]) -> &'static str {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else {
        "jpg"
    }
}

/// Writes the `rows` into the library: one logged edit per book, with the source of each field
/// recorded. The values come from `review.results`, the book as it is now is the base, so edits
/// made since the lookup are not lost.
pub fn apply_rows(
    lib: &mut Library,
    store: &SqliteStore,
    review: &Review,
    rows: &[Row],
    now: i64,
) -> Applied {
    let mut out = Applied::default();
    let mut books: Vec<u64> = rows.iter().map(|r| r.book).collect();
    books.sort_unstable();
    books.dedup();
    for book in books {
        let mine: Vec<&Row> = rows.iter().filter(|r| r.book == book).collect();
        let Some(found) = review
            .results
            .iter()
            .find(|r| r.book == book)
            .and_then(|r| r.found.as_ref())
        else {
            continue;
        };
        let Some(before) = lib.get(book).map(|b| b.meta.clone()) else {
            continue;
        };
        let fields: Vec<&str> = mine.iter().map(|r| r.field).collect();
        let after = merge_fields(&before, &found.meta, &fields);
        let step = (|| -> Result<()> {
            if fields.contains(&"cover") {
                if let Some(data) = &found.cover {
                    let path = lib.get(book).map(|b| lib.book_path(b));
                    if let Some(path) = path {
                        let mut ob = crate::core::book::OpenBook::open(&path)?;
                        ob.set_cover(data.clone(), image_ext(data))?;
                        ob.archive.save(&path)?;
                    }
                }
            }
            lib.update_metadata(book, after.clone())?;
            Ok(())
        })();
        match step {
            Ok(()) => {
                let sources: Vec<(String, String)> = mine
                    .iter()
                    .map(|r| (r.field.to_string(), r.source.clone()))
                    .collect();
                crate::library_health::record_edit_at(store, book, &before, &after, &sources, now);
                if fields.contains(&"cover") {
                    let _ = store.set_provenance(
                        book,
                        &["cover"],
                        mine.iter()
                            .find(|r| r.field == "cover")
                            .map_or("online", |r| r.source.as_str()),
                        now,
                    );
                }
                out.books += 1;
                out.ops += 1;
                out.fields += fields.len();
                out.touched
                    .push((book, fields.iter().map(|f| f.to_string()).collect()));
            }
            Err(e) => out.errors.push(format!("{}: {e:#}", before.title)),
        }
    }
    out
}

/// Takes back a batch through the operation log. The cover of a book stays as it is; metadata
/// fields go back and their recorded source is forgotten.
pub fn undo_applied(lib: &mut Library, store: &SqliteStore, applied: &Applied) -> Result<usize> {
    let mut n = 0;
    for _ in 0..applied.ops {
        if lib.undo_last()?.is_some() {
            n += 1;
        }
    }
    for (book, fields) in &applied.touched {
        let f: Vec<&str> = fields.iter().map(String::as_str).collect();
        let _ = store.clear_provenance(*book, &f);
    }
    Ok(n)
}

/// Looks the books up one after the other (call from a worker thread). `progress(done, total)`
/// is called after every book; setting `cancel` stops after the current one.
pub fn run_batch(
    items: Vec<(u64, Metadata, bool)>,
    fetcher: Arc<dyn Fetcher>,
    plugins: &crate::plugins::Loaded,
    pause_ms: u64,
    cancel: &AtomicBool,
    progress: &dyn Fn(usize, usize),
) -> Vec<BookResult> {
    let total = items.len();
    let mut out = Vec::new();
    for (i, (book, md, has_cover)) in items.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let (found, error) = match mf::fetch_with(&md, fetcher.clone(), plugins) {
            Ok(f) => (f, None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        out.push(BookResult {
            book,
            title: md.title.clone(),
            current: md,
            has_cover,
            cover_px: None,
            found,
            error,
        });
        progress(i + 1, total);
        if pause_ms > 0 && i + 1 < total {
            std::thread::sleep(std::time::Duration::from_millis(pause_ms));
        }
    }
    out
}

/// The line of the health display: `Open Library ok · Google Books slow · Wikidata failing`.
pub fn health_line(board: &bookx_sources::health::HealthBoard) -> String {
    let reports = board.snapshot_now();
    if reports.is_empty() {
        return tr("Sources: not used yet");
    }
    trf(
        "Sources: {}",
        &[&(reports
            .iter()
            .map(|r| r.describe())
            .collect::<Vec<_>>()
            .join(" · "))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::opf::Identifier;

    fn found(conf: f64) -> Fetched {
        Fetched {
            meta: Metadata {
                title: "Dune".into(),
                authors: vec!["Frank Herbert".into()],
                publisher: "Chilton".into(),
                published: "1965".into(),
                language: "en".into(),
                series: "Dune Chronicles".into(),
                series_index: 1.0,
                identifiers: vec![Identifier {
                    scheme: "isbn".into(),
                    value: "9780441172719".into(),
                    xml_id: None,
                }],
                ..Default::default()
            },
            cover: Some(vec![0xFF, 0xD8, 1, 2, 3]),
            confidence: conf,
            sources: vec!["openlibrary".into(), "google".into()],
            field_sources: [
                ("publisher".to_string(), "google".to_string()),
                ("cover".to_string(), "openlibrary".to_string()),
            ]
            .into(),
            warnings: vec![],
        }
    }

    fn result(book: u64, current: Metadata, f: Option<Fetched>) -> BookResult {
        BookResult {
            book,
            title: current.title.clone(),
            current,
            has_cover: false,
            cover_px: None,
            found: f,
            error: None,
        }
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        bookx_sources::cover::generate_cover("Dune", "Frank Herbert", "", w, h).unwrap()
    }

    fn cover_row_of(has: bool, now: Option<(u32, u32)>, new: Option<(u32, u32)>) -> Row {
        let mut f = found(0.9);
        f.cover = new.map(|(w, h)| png(w, h));
        let mut r = result(1, weak("Dune"), Some(f));
        r.has_cover = has;
        r.cover_px = now;
        rows_for(&r)
            .into_iter()
            .find(|r| r.field == "cover")
            .unwrap()
    }

    #[test]
    fn a_much_larger_cover_replaces_a_poor_one_as_a_safe_change() {
        // PP-G-28: the book has a 150x200 cover, the source offers 800x1200
        let r = cover_row_of(true, Some((150, 200)), Some((800, 1200)));
        assert!(r.safe, "an upgrade of a low-resolution cover is safe");
        assert!(
            r.current.contains("150 × 200 px") && r.current.contains("very small"),
            "{}",
            r.current
        );
        assert!(r.proposed.contains("800 × 1200 px"), "{}", r.proposed);
    }

    #[test]
    fn a_good_cover_is_never_replaced_silently() {
        let r = cover_row_of(true, Some((800, 1200)), Some((1600, 2400)));
        assert!(!r.safe, "a cover that is already good needs a decision");
        let r = cover_row_of(true, Some((400, 500)), Some((410, 520)));
        assert!(!r.safe, "barely bigger is no upgrade");
        let r = cover_row_of(true, None, Some((800, 1200)));
        assert!(!r.safe && r.current == "has a cover");
        let r = cover_row_of(false, None, Some((800, 1200)));
        assert!(r.safe, "no cover yet: fill it");
    }

    fn weak(title: &str) -> Metadata {
        Metadata {
            title: title.into(),
            authors: vec!["Frank Herbert".into()],
            ..Default::default()
        }
    }

    #[test]
    fn the_settings_default_to_asking_and_old_configs_load() {
        let p = MetadataPrefs::default();
        assert!(!p.auto_apply_safe && !p.preflight && p.conversion_report);
        let loaded: MetadataPrefs = serde_json::from_str("{\"preflight\": true}").unwrap();
        assert!(loaded.preflight && !loaded.auto_apply_safe && loaded.conversion_report);
    }

    #[test]
    fn rows_show_current_and_proposed_with_source_and_confidence() {
        let rows = rows_for(&result(1, weak("Dune"), Some(found(0.9))));
        let fields: Vec<_> = rows.iter().map(|r| r.field).collect();
        assert_eq!(
            fields,
            vec![
                "series",
                "publisher",
                "published",
                "language",
                "identifiers",
                "cover"
            ]
        );
        let s = &rows[0];
        assert_eq!(s.current, "");
        assert_eq!(s.proposed, "Dune Chronicles #1");
        assert_eq!(
            s.source, "openlibrary, google",
            "no per-field source: all of them"
        );
        let by_field = |f: &str| rows.iter().find(|r| r.field == f).unwrap().source.clone();
        assert_eq!(
            by_field("publisher"),
            "google",
            "the table names the source of each field"
        );
        assert_eq!(by_field("cover"), "openlibrary");
        assert_eq!(s.confidence, 0.9);
        assert!(rows.iter().all(|r| r.safe && !r.accepted));
    }

    #[test]
    fn a_field_that_already_has_a_value_is_not_safe() {
        let mut cur = weak("Dune");
        cur.publisher = "Ace".into();
        let rows = rows_for(&result(1, cur, Some(found(0.9))));
        let p = rows.iter().find(|r| r.field == "publisher").unwrap();
        assert_eq!(
            (p.current.as_str(), p.proposed.as_str()),
            ("Ace", "Chilton")
        );
        assert!(!p.safe);
        assert!(rows.iter().find(|r| r.field == "language").unwrap().safe);
    }

    #[test]
    fn nothing_found_or_nothing_new_gives_no_rows() {
        assert!(rows_for(&result(1, weak("Dune"), None)).is_empty());
        let mut f = found(0.9);
        f.cover = None;
        let mut cur = f.meta.clone();
        cur.identifiers = f.meta.identifiers.clone();
        assert!(rows_for(&result(1, cur, Some(f))).is_empty());
    }

    #[test]
    fn accepting_all_by_field_by_book_and_only_safe() {
        let mut cur2 = weak("Dune Messiah");
        cur2.publisher = "Mine".into();
        let mut r = Review::new(vec![
            result(1, weak("Dune"), Some(found(0.9))),
            result(2, cur2, Some(found(0.7))),
        ]);
        assert_eq!(r.accepted(), 0, "nothing is accepted by default");
        r.accept_field("language", true);
        assert_eq!(r.accepted(), 2);
        r.accept_all(false);
        r.accept_field("publisher", true);
        assert!(
            r.rows
                .iter()
                .filter(|x| x.accepted)
                .all(|x| x.field == "publisher")
        );
        r.accept_safe();
        assert!(r.rows.iter().all(|x| x.accepted == x.safe));
        assert!(
            r.rows
                .iter()
                .any(|x| x.book == 2 && x.field == "publisher" && !x.accepted),
            "the publisher the user typed is not overwritten by 'safe'"
        );
        let before = r.accepted();
        r.toggle(0);
        assert_ne!(r.accepted(), before);
        assert_eq!(r.fields_present()[0], "title");
        assert!(r.summary().starts_with("2 of 2 books found"));
    }

    #[test]
    fn only_the_taken_fields_are_merged() {
        let mut cur = weak("Dune");
        cur.publisher = "Ace".into();
        let m = merge_fields(&cur, &found(0.9).meta, &["language", "series"]);
        assert_eq!(m.language, "en");
        assert_eq!(
            (m.series.as_str(), m.series_index),
            ("Dune Chronicles", 1.0)
        );
        assert_eq!(m.publisher, "Ace");
        assert!(m.published.is_empty());
        let t = merge_fields(&cur, &found(0.9).meta, &["authors", "title"]);
        assert_eq!(t.author_sort, "Herbert, Frank");
    }

    #[test]
    fn failures_and_misses_are_counted() {
        let mut bad = result(1, weak("A"), None);
        bad.error = Some("all metadata sources failed".into());
        let r = Review::new(vec![bad, result(2, weak("B"), None)]);
        assert_eq!(r.failures().len(), 1);
        assert_eq!(r.not_found(), 1);
        assert!(r.summary().contains("1 failed"));
    }

    fn library_with_two_books() -> (
        Library,
        std::path::PathBuf,
        crate::library_health::LibraryGuard,
    ) {
        let d = std::env::temp_dir().join(format!("bookx-bulk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let (mut lib, g, _) = crate::library_health::open(&d.join("lib")).unwrap();
        for (i, t) in ["Dune", "Other Tale"].iter().enumerate() {
            std::fs::write(
                d.join(format!("{i}.md")),
                format!("---\ntitle: \"{t}\"\nauthor: \"Frank Herbert\"\n---\n# a\n\ntext\n"),
            )
            .unwrap();
            lib.add_path(&d.join(format!("{i}.md")), false).unwrap();
        }
        (lib, d, g)
    }

    #[test]
    fn applying_writes_accepted_rows_records_sources_and_can_be_undone() {
        let (mut lib, d, guard) = library_with_two_books();
        let store = &*guard.store;
        let ids: Vec<u64> = lib.books.iter().map(|b| b.id).collect();
        let results: Vec<BookResult> = ids
            .iter()
            .map(|id| {
                let b = lib.get(*id).unwrap();
                result(*id, b.meta.clone(), Some(found(0.8)))
            })
            .collect();
        let mut review = Review::new(results);
        review.accept_field("publisher", true);
        review.accept_field("series", true);
        let rows = review.take_accepted();
        assert_eq!(rows.len(), 4);
        let ops_before = lib.history().len();
        let a = apply_rows(&mut lib, store, &review, &rows, 1000);
        assert_eq!((a.books, a.fields, a.ops), (2, 4, 2));
        assert!(a.errors.is_empty(), "{:?}", a.errors);
        assert_eq!(
            lib.history().len(),
            ops_before + 2,
            "one logged edit per book"
        );
        for id in &ids {
            let m = &lib.get(*id).unwrap().meta;
            assert_eq!(m.publisher, "Chilton");
            assert_eq!(m.series, "Dune Chronicles");
            assert!(m.published.is_empty(), "published was not accepted");
            let p = store.provenance(*id);
            assert!(
                p.iter()
                    .any(|f| f.field == "publisher" && f.source == "google")
            );
            assert!(!p.iter().any(|f| f.field == "published"));
        }
        let n = undo_applied(&mut lib, store, &a).unwrap();
        assert_eq!(n, 2);
        for id in &ids {
            assert_eq!(lib.get(*id).unwrap().meta.publisher, "");
            assert!(store.provenance(*id).iter().all(|f| f.field != "publisher"));
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_batch_runs_over_a_mock_and_reports_progress_and_cancels() {
        let mock = std::env::temp_dir().join(format!("bookx-bulk-mock-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&mock).unwrap();
        std::fs::write(
            mock.join("openlibrary.json"),
            r#"{"numFound":1,"docs":[{"key":"/works/OL1W","title":"Dune","author_name":["Frank Herbert"],"first_publish_year":1965,"publisher":["Chilton"],"isbn":["9780441172719"],"language":["eng"]}]}"#,
        )
        .unwrap();
        let fetcher: Arc<dyn Fetcher> = Arc::new(mf::mock_from(&mock));
        let items = vec![(1, weak("Dune"), false), (2, weak("Dune"), false)];
        let seen = std::sync::Mutex::new(Vec::new());
        let plugins = crate::plugins::Loaded::default();
        let out = run_batch(
            items.clone(),
            fetcher.clone(),
            &plugins,
            0,
            &AtomicBool::new(false),
            &|d, t| seen.lock().unwrap().push((d, t)),
        );
        assert_eq!(out.len(), 2);
        assert_eq!(*seen.lock().unwrap(), vec![(1, 2), (2, 2)]);
        assert!(out.iter().all(|r| r.found.is_some()), "{:?}", out[0].error);
        let rows = rows_for(&out[0]);
        assert!(
            rows.iter()
                .any(|r| r.field == "publisher" && r.proposed == "Chilton")
        );
        let stop = AtomicBool::new(true);
        assert!(run_batch(items, fetcher, &plugins, 0, &stop, &|_, _| {}).is_empty());
        let _ = std::fs::remove_dir_all(mock);
    }
}
