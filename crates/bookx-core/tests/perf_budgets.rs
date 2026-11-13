//! Performance budgets ("instant by contract"). Each test fails when an operation gets slower than
//! its budget, so regressions show up in `cargo test`.
//!
//! Budgets are for release builds; debug builds get `DEBUG_FACTOR` times more room. Set
//! `BOOKX_PERF_SCALE=2` (or 0.5) to scale every budget for slower or faster machines.
//! Every test builds its own data in memory or in its own temp dir, so they run in parallel.

use std::time::{Duration, Instant};

use bookx_core::book::OpenBook;
use bookx_core::convert::{create_epub, Chapter};
use bookx_core::library::{prepare_import, Book, Library, SortKey};
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
