//! Performance budgets for OPDS feed generation. Budgets are for release builds; debug builds get 30x room and
//! `BOOKX_PERF_SCALE` scales every budget (like `bookx-core/tests/perf_budgets.rs`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bookx_core::library::Book;
use bookx_core::opf::Metadata;
use bookx_serve::opds::{OpdsConfig, route};
use chrono::Utc;

fn budget(ms: u64) -> Duration {
    let debug = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    Duration::from_secs_f64(ms as f64 / 1000.0 * debug * scale)
}

fn timed<T>(name: &str, ms: u64, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    let took = t.elapsed();
    assert!(
        took <= budget(ms),
        "{name} took {took:?}, budget {:?}",
        budget(ms)
    );
    out
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
        language: "en".into(),
        description: "A description of the book. ".repeat(8),
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
        formats: vec![],
        resources: vec![],
        drm: String::new(),
        custom: Default::default(),
        marked: false,
    }
}

fn segs(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}

#[test]
fn opds_feeds_of_a_10k_library() {
    let books: Vec<Book> = (0..10_000).map(synthetic_book).collect();
    let cfg = OpdsConfig::default();
    let none = HashMap::new();

    let doc = timed("root feed", 100, || route(&cfg, &books, &[], &none)).unwrap();
    assert!(doc.body.contains("<feed"));
    let doc = timed("all, first page", 150, || {
        route(&cfg, &books, &segs(&["all"]), &none)
    })
    .unwrap();
    assert_eq!(doc.body.matches("<entry>").count(), cfg.page_size);
    let mut last = HashMap::new();
    last.insert("page".to_string(), "200".to_string());
    timed("all, last page", 150, || {
        route(&cfg, &books, &segs(&["all"]), &last)
    })
    .unwrap();
    timed("new", 150, || route(&cfg, &books, &segs(&["new"]), &none)).unwrap();
    timed("authors navigation", 150, || {
        route(&cfg, &books, &segs(&["authors"]), &none)
    })
    .unwrap();
    timed("series navigation", 150, || {
        route(&cfg, &books, &segs(&["series"]), &none)
    })
    .unwrap();
    let mut q = HashMap::new();
    q.insert(
        "q".to_string(),
        "author:\"Author 7\" tag:fiction".to_string(),
    );
    let doc = timed("search", 150, || {
        route(&cfg, &books, &segs(&["search"]), &q)
    })
    .unwrap();
    assert!(doc.body.contains("Author 7"));
}
