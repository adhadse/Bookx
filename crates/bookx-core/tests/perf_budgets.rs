//! Performance budgets ("instant by contract"). Each test fails when an operation gets slower than
//! its budget, so regressions show up in `cargo test`.
//!
//! Budgets are for release builds; debug builds get `DEBUG_FACTOR` times more room. Set
//! `BOOKX_PERF_SCALE=2` (or 0.5) to scale every budget for slower or faster machines.
//! Every test builds its own data in memory or in its own temp dir, so they run in parallel.

use std::time::{Duration, Instant};

use bookx_core::book::OpenBook;
use bookx_core::convert::{Chapter, create_epub};
use bookx_core::library::{Book, Library, SortKey, prepare_import};
use bookx_core::opf::Metadata;
use chrono::Utc;

const DEBUG_FACTOR: f64 = 30.0;

fn budget(release_ms: u64) -> Duration {
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let f = if cfg!(debug_assertions) {
        DEBUG_FACTOR
    } else {
        1.0
    };
    Duration::from_micros((release_ms as f64 * 1000.0 * f * scale) as u64)
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let v = f();
    (v, t.elapsed())
}

fn check(what: &str, took: Duration, max: Duration) {
    assert!(
        took <= max,
        "{what}: took {took:?}, budget {max:?} (BOOKX_PERF_SCALE adjusts budgets)"
    );
}

fn synthetic_book(i: u64) -> Book {
    let meta = Metadata {
        title: format!("Book number {i}"),
        title_sort: format!("book number {i}"),
        authors: vec![format!("Author {}", i % 500)],
        author_sort: format!("author {}", i % 500),
        tags: vec![format!("tag{}", i % 40), "fiction".into()],
        series: format!("Series {}", i % 300),
        series_index: (i % 12) as f64,
        rating: (i % 11) as u8,
        language: "en".into(),
        ..Default::default()
    };
    Book {
        id: i + 1,
        file: format!("a/b/{i}.epub"),
        cover: None,
        meta,
        added: Utc::now(),
        modified: Utc::now(),
        last_read: None,
        size: 1000 + i,
        progress: 0.0,
        position: None,
        bookmarks: vec![],
        highlights: vec![],
        formats: Vec::new(),
        resources: Vec::new(),
        drm: String::new(),
        custom: Default::default(),
        marked: false,
    }
}

fn library_of(n: u64, dir: &std::path::Path) -> Library {
    let mut lib = Library::in_memory(dir).unwrap();
    lib.books = (0..n).map(synthetic_book).collect();
    lib
}

fn sample_epub(chapters: usize) -> Vec<u8> {
    let md = Metadata {
        title: "Perf".into(),
        authors: vec!["Tester".into()],
        ..Default::default()
    };
    let ch: Vec<Chapter> = (0..chapters)
        .map(|i| Chapter {
            title: format!("Chapter {i}"),
            body: "<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit.</p>".repeat(200),
        })
        .collect();
    create_epub(&md, &ch, None).to_bytes().unwrap()
}

#[test]
fn search_10k_books_is_instant() {
    let dir = tempdir("search");
    let lib = library_of(10_000, &dir);
    let (hits, took) =
        timed(|| lib.search("author:=\"Author 42\" tag:fiction", SortKey::Title, true));
    assert_eq!(hits.len(), 20);
    check("search over 10 000 books", took, budget(50));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn sorted_listing_of_10k_books_is_instant() {
    let dir = tempdir("sort");
    let lib = library_of(10_000, &dir);
    let (ids, took) = timed(|| lib.search("", SortKey::Author, true));
    assert_eq!(ids.len(), 10_000);
    check("sorting 10 000 books", took, budget(80));
    std::fs::remove_dir_all(dir).ok();
}

/// Counts how many books each commit receives.
#[derive(Clone, Default)]
struct Counting {
    inner: bookx_core::library::MemoryBackend,
    commits: std::sync::Arc<std::sync::Mutex<Vec<(usize, usize)>>>,
}

impl bookx_core::library::CatalogBackend for Counting {
    fn load(&mut self) -> anyhow::Result<(u64, Vec<Book>)> {
        bookx_core::library::CatalogBackend::load(&mut self.inner)
    }
    fn commit(&mut self, next_id: u64, upserts: &[&Book], removed: &[u64]) -> anyhow::Result<()> {
        self.commits
            .lock()
            .unwrap()
            .push((upserts.len(), removed.len()));
        bookx_core::library::CatalogBackend::commit(&mut self.inner, next_id, upserts, removed)
    }
}

#[test]
fn saving_only_writes_what_changed() {
    let dir = tempdir("save");
    let backend = Counting::default();
    let mut lib = Library::with_backend(&dir, Box::new(backend.clone())).unwrap();
    lib.books = (0..10_000).map(synthetic_book).collect();
    lib.save().unwrap();
    assert_eq!(backend.commits.lock().unwrap().last(), Some(&(10_000, 0)));

    // nothing changed: no commit at all, and fast
    let (_, took) = timed(|| lib.save().unwrap());
    check("saving 10 000 unchanged books", took, budget(150));
    assert_eq!(backend.commits.lock().unwrap().len(), 1);

    // one book edited and one removed: the commit carries just those
    lib.books[17].progress = 0.5;
    lib.books.remove(99);
    let (_, took) = timed(|| lib.save().unwrap());
    check(
        "saving one edit and one removal among 10 000 books",
        took,
        budget(150),
    );
    assert_eq!(backend.commits.lock().unwrap().last(), Some(&(1, 1)));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn catalogue_of_10k_books_loads_fast() {
    let dir = tempdir("catalogue");
    let backend = bookx_core::library::MemoryBackend::new();
    let mut lib = Library::with_backend(&dir, Box::new(backend.clone())).unwrap();
    lib.books = (0..10_000).map(synthetic_book).collect();
    lib.save().unwrap();
    let (lib, took) = timed(|| Library::with_backend(&dir, Box::new(backend.clone())).unwrap());
    assert_eq!(lib.books.len(), 10_000);
    check("opening a 10 000 book catalogue", took, budget(400));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn opening_a_book_is_fast() {
    let bytes = sample_epub(40);
    let (book, took) = timed(|| {
        OpenBook::from_archive(bookx_core::archive::Archive::from_bytes(bytes).unwrap()).unwrap()
    });
    assert_eq!(book.spine_paths().len(), 40);
    check("opening a 40 chapter EPUB", took, budget(100));
}

#[test]
fn importing_books_scales_across_cores() {
    let dir = tempdir("import");
    let bytes = sample_epub(5);
    let files: Vec<_> = (0..200)
        .map(|i| {
            let p = dir.join(format!("b{i}.epub"));
            std::fs::write(&p, &bytes).unwrap();
            p
        })
        .collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = files.len().div_ceil(threads);
    let (ok, took) = timed(|| {
        std::thread::scope(|s| {
            files
                .chunks(chunk)
                .map(|c| s.spawn(move || c.iter().filter(|p| prepare_import(p).is_ok()).count()))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| h.join().unwrap())
                .sum::<usize>()
        })
    });
    assert_eq!(ok, 200);
    // 1000 books in 60 s is the product goal; 200 books must stay well inside a fifth of that
    check("preparing 200 imports in parallel", took, budget(12_000));
    std::fs::remove_dir_all(dir).ok();
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "bookx-perf-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn books_for_ids_keeps_order_and_is_linear() {
    let dir = tempdir("books-for");
    let mut lib = Library::in_memory(&dir).unwrap();
    lib.books = (0..50_000).map(synthetic_book).collect();
    let mut ids: Vec<u64> = lib.books.iter().map(|b| b.id).collect();
    ids.reverse();
    ids.push(u64::MAX); // unknown ids are left out
    let (got, took) = timed(|| lib.books_for(&ids));
    assert_eq!(got.len(), 50_000);
    assert_eq!(got[0].id, lib.books[49_999].id);
    assert_eq!(got[49_999].id, lib.books[0].id);
    // a `get` per id takes seconds here (quadratic); one pass takes milliseconds
    check("books for 50 000 ids", took, budget(100));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_get_per_selected_book_is_not_quadratic() {
    // Select All on 100 000 books: the details panel, the shelves and the devices ask for every selected
    // book by id; `get` used to scan the whole list for each (10^10 steps)
    let dir = tempdir("get-per-id");
    let mut lib = Library::in_memory(&dir).unwrap();
    lib.books = (0..100_000).map(synthetic_book).collect();
    let ids: Vec<u64> = lib.books.iter().map(|b| b.id).collect();
    let (n, took) = timed(|| ids.iter().filter(|id| lib.get(**id).is_some()).count());
    assert_eq!(n, 100_000);
    check("get for 100 000 ids", took, budget(100));
    let (n, took) = timed(|| {
        let mut n = 0;
        for id in &ids {
            if let Some(b) = lib.get_mut(*id) {
                b.progress = 0.5;
                n += 1;
            }
        }
        n
    });
    assert_eq!(n, 100_000);
    check("get_mut for 100 000 ids", took, budget(100));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn get_finds_books_in_any_order_and_misses_cleanly() {
    let dir = tempdir("get-order");
    let mut lib = Library::in_memory(&dir).unwrap();
    lib.books = (0..50).map(synthetic_book).collect();
    let ids: Vec<u64> = lib.books.iter().map(|b| b.id).collect();
    lib.books.remove(10); // a deleted book leaves a gap
    assert!(lib.get(ids[10]).is_none());
    assert_eq!(lib.get(ids[11]).map(|b| b.id), Some(ids[11]));
    lib.books.reverse(); // not in id order: the scan answers
    for id in ids.iter().filter(|id| **id != ids[10]) {
        assert_eq!(lib.get(*id).map(|b| b.id), Some(*id));
        assert_eq!(lib.get_mut(*id).map(|b| b.id), Some(*id));
    }
    assert!(lib.get(u64::MAX).is_none());
    std::fs::remove_dir_all(dir).ok();
}

/// A book like `build-aux/make-big-epub.py OUT 20000`: 20 files of one long line each, a nested EPUB 3
/// navigation document with `entries` entries.
fn big_toc_epub(entries: usize) -> Vec<u8> {
    use std::io::Write;
    let parts = 20;
    let per_part = entries / parts;
    let mut nav = String::new();
    let mut files: Vec<(String, String)> = Vec::new();
    for p in 0..parts {
        let mut body = format!("<h1 id=\"p{p}\">Part {p}</h1>");
        let mut lis = String::new();
        for s in 0..per_part {
            body.push_str(&format!(
                "<h3 id=\"s{p}_{s}\">Section {s}</h3><p>Paragraph of section {p}.{s}. The reader opens this book to test the sidebar.</p>"
            ));
            lis.push_str(&format!(
                "<li><a href=\"part{p}.xhtml#s{p}_{s}\">Section {p}.{s}</a></li>"
            ));
        }
        nav.push_str(&format!(
            "<li><a href=\"part{p}.xhtml\">Part {p}</a><ol>{lis}</ol></li>"
        ));
        files.push((
            format!("part{p}.xhtml"),
            format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Part</title></head><body>{body}</body></html>"),
        ));
    }
    let manifest: String = (0..parts)
        .map(|i| {
            format!(
                "<item id=\"f{i}\" href=\"part{i}.xhtml\" media-type=\"application/xhtml+xml\"/>"
            )
        })
        .collect();
    let spine: String = (0..parts)
        .map(|i| format!("<itemref idref=\"f{i}\"/>"))
        .collect();
    let opf = format!(
        "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"id\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"id\">big</dc:identifier><dc:title>Big</dc:title><dc:language>en</dc:language></metadata><manifest><item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>{manifest}</manifest><spine>{spine}</spine></package>"
    );
    let navdoc = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Contents</title></head><body><nav epub:type=\"toc\"><ol>{nav}</ol></nav></body></html>"
    );
    let container = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut out);
        let opts = zip::write::SimpleFileOptions::default();
        z.start_file("mimetype", opts).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.start_file("META-INF/container.xml", opts).unwrap();
        z.write_all(container.as_bytes()).unwrap();
        z.start_file("OEBPS/content.opf", opts).unwrap();
        z.write_all(opf.as_bytes()).unwrap();
        z.start_file("OEBPS/nav.xhtml", opts).unwrap();
        z.write_all(navdoc.as_bytes()).unwrap();
        for (n, c) in &files {
            z.start_file(format!("OEBPS/{n}"), opts).unwrap();
            z.write_all(c.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    out.into_inner()
}

#[test]
fn a_book_with_20000_contents_entries_opens_and_lists_fast() {
    let bytes = big_toc_epub(20_000);
    let (book, took) = timed(|| {
        OpenBook::from_archive(bookx_core::archive::Archive::from_bytes(bytes).unwrap()).unwrap()
    });
    check("opening a 20 000 entry EPUB", took, budget(250));
    let (flat, took) = timed(|| bookx_core::toc::flatten(&book.toc()));
    assert!(flat.len() >= 20_000, "{} entries", flat.len());
    check(
        "reading and flattening 20 000 contents entries",
        took,
        budget(400),
    );
    // the editor asks for the same notes of every file: the semantics are read once, not once per file
    let (_, took) = timed(|| book.spine_paths().len());
    check("listing the spine of a 20 000 entry book", took, budget(20));
}

#[test]
fn custom_column_search_sort_and_regex_over_100k_books_are_fast() {
    use bookx_core::custom::{ColumnDef, ColumnKind};
    let dir = tempdir("custom100k");
    let mut lib = library_of(100_000, &dir);
    lib.set_columns(vec![
        ColumnDef {
            label: "pages".into(),
            name: "Pages".into(),
            kind: ColumnKind::Integer,
            ..Default::default()
        },
        ColumnDef {
            label: "mood".into(),
            name: "Mood".into(),
            kind: ColumnKind::Text,
            ..Default::default()
        },
    ])
    .unwrap();
    for (i, b) in lib.books.iter_mut().enumerate() {
        b.custom.insert("pages".into(), (i % 1000).to_string());
        if i % 7 == 0 {
            b.custom.insert("mood".into(), "calm".into());
        }
    }
    let (hits, took) = timed(|| lib.search("#pages:>=990 #mood:calm", SortKey::Title, true));
    assert!(!hits.is_empty());
    check(
        "a custom column search over 100 000 books",
        took,
        budget(1500),
    );
    let (hits, took) = timed(|| {
        lib.search(
            "(#mood:calm OR tag:fiction) -#pages:<500",
            SortKey::Title,
            true,
        )
    });
    assert!(!hits.is_empty());
    check("a grouped search over 100 000 books", took, budget(1500));
    let (hits, took) = timed(|| {
        lib.search(
            "title:~^Book\\s*9[0-9]+$ added:>=2000-01-01",
            SortKey::Title,
            true,
        )
    });
    assert!(hits.len() < 100_000);
    check(
        "a regular expression search over 100 000 books",
        took,
        budget(2500),
    );
    let mut ids: Vec<u64> = lib.books.iter().map(|b| b.id).collect();
    let ((), took) = timed(|| lib.sort_by_custom(&mut ids, "pages", true));
    check(
        "sorting 100 000 books by a custom column",
        took,
        budget(600),
    );
    std::fs::remove_dir_all(dir).ok();
}
