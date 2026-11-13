//! Performance budgets for the store. Budgets are for release builds; debug builds get 30x room and
//! `BOOKX_PERF_SCALE` scales every budget (like `bookx-core/tests/perf_budgets.rs`).

mod common;

use std::time::{Duration, Instant};

use bookx_store::SqliteStore;
use common::*;

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

const VOCAB: &[&str] = &[
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango",
];

fn word(r: &mut Rng) -> String {
    format!("{}{}", r.pick(VOCAB), r.below(500))
}

#[test]
fn ten_thousand_books_put_and_metadata_search() {
    let mut r = Rng(12345);
    let books: Vec<_> = (1..=10_000u64)
        .map(|id| {
            let title = format!("{} {} {}", word(&mut r), word(&mut r), word(&mut r));
            let mut b = book(id, &title, &format!("{} {}", word(&mut r), word(&mut r)));
            b.meta.tags = vec![word(&mut r), word(&mut r)];
            b.meta.description = "a description ".repeat(10);
            b
        })
        .collect();
    let mut needle = books[4242].clone();
    needle.meta.title = "Zanzibarlike Needle".into();
    let mut books = books;
    books[4242] = needle;

    let s = SqliteStore::open_in_memory().unwrap();
    // loading budget is generous: 10k inserts incl. two index writes each
    timed("put 10_000 books", 3000, || s.put_many(&books).unwrap());
    assert_eq!(s.len(), 10_000);

    let hits = timed("metadata search", 50, || s.search_ids("zanzibarlike"));
    assert_eq!(hits, vec![4243]);
    let hits = timed("metadata search (common word)", 50, || s.search_ids("echo"));
    assert!(!hits.is_empty());
    timed("sorted ids", 50, || {
        s.ids_sorted(bookx_store::Order::Title, true)
    });
}

#[test]
fn text_search_over_two_thousand_texts() {
    let s = SqliteStore::open_in_memory().unwrap();
    let books: Vec<_> = (1..=2000u64).map(|i| book(i, "t", "a")).collect();
    s.put_many(&books).unwrap();
    let mut r = Rng(777);
    let texts: Vec<(u64, String)> = (1..=2000u64)
        .map(|id| {
            let mut t: Vec<String> = (0..300).map(|_| word(&mut r)).collect();
            if id == 1234 {
                t.push("quetzalcoatlus".into());
            }
            (id, t.join(" "))
        })
        .collect();
    timed("index 2_000 texts", 5000, || {
        s.index_book_texts(&texts).unwrap()
    });
    let hits = timed("rare word text search", 50, || {
        s.search_text("quetzalcoatlus", 20).unwrap()
    });
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, 1234);
    let hits = timed("common word text search", 50, || {
        s.search_text("alpha1", 20).unwrap()
    });
    assert!(hits.len() <= 20);
}

fn library_of_10k(root: &std::path::Path) -> bookx_store::OpenedLibrary {
    let mut o = bookx_store::open_library(root).unwrap();
    o.library.books = (1..=10_000u64)
        .map(|id| book(id, &format!("Title {id}"), &format!("Author {}", id % 500)))
        .collect();
    o
}

fn lib_tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bookx-perf-lib-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn saving_a_10k_library_the_first_time_is_one_transaction() {
    let d = lib_tmp("first");
    let o = library_of_10k(&d.join("lib"));
    timed("first save of 10 000 books", 3000, || {
        o.library.save().unwrap()
    });
    assert_eq!(o.store.len(), 10_000);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn saving_one_edit_among_10k_books_is_fast() {
    let d = lib_tmp("edit");
    let mut o = library_of_10k(&d.join("lib"));
    o.library.save().unwrap();
    // reading progress is saved on page turns: this is the hot path
    o.library.books[4_321].progress = 0.5;
    timed("saving one edited book among 10 000", 150, || {
        o.library.save().unwrap()
    });
    // and a save with nothing to do
    timed("saving 10 000 unchanged books", 150, || {
        o.library.save().unwrap()
    });
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn reopening_a_10k_library_is_fast() {
    let d = lib_tmp("reopen");
    let root = d.join("lib");
    {
        let o = library_of_10k(&root);
        o.library.save().unwrap();
    }
    let o = timed("opening a 10 000 book library", 1000, || {
        bookx_store::open_library(&root).unwrap()
    });
    assert_eq!(o.library.books.len(), 10_000);
    let _ = std::fs::remove_dir_all(d);
}

// -- 100 000 books (roadmap PP-G-12) ---------------------------------------------------------------
//
// Budgets are for release builds and kept near what the code does with some room; the point is to
// catch an accidental O(n^2), not to race.

const N100K: u64 = 100_000;

fn synthetic_book(id: u64, r: &mut Rng) -> bookx_core::library::Book {
    let title = format!("{} {} {}", word(r), word(r), id);
    let mut b = book(id, &title, &format!("{} {}", word(r), word(r)));
    b.meta.tags = vec![word(r)];
    b.meta.series = format!("Series {}", id % 3000);
    b.meta.series_index = (id % 12) as f64;
    b.meta.description = "a description of the book ".repeat(4);
    b
}

#[test]
fn a_hundred_thousand_book_library() {
    let d = lib_tmp("100k");
    let root = d.join("lib");
    let mut r = Rng(2026);
    {
        let mut o = bookx_store::open_library(&root).unwrap();
        o.library.books = (1..=N100K).map(|id| synthetic_book(id, &mut r)).collect();
        o.library.books[77_777].meta.title = "Zanzibarlike Needle".into();
        timed("first save of 100 000 books", 8_000, || {
            o.library.save().unwrap()
        });
        assert_eq!(o.store.len(), N100K as usize);

        timed("one progress save among 100 000 books", 400, || {
            o.library.books[54_321].progress = 0.5;
            o.library.save().unwrap()
        });
        timed("saving 100 000 unchanged books", 400, || {
            o.library.save().unwrap()
        });
        let hits = timed("metadata search among 100 000", 100, || {
            o.store.search_ids("zanzibarlike")
        });
        assert_eq!(hits, vec![77_778]);
        let ids = timed("title-sorted ids of 100 000", 500, || {
            o.store.ids_sorted(bookx_store::Order::Title, true)
        });
        assert_eq!(ids.len(), N100K as usize);
        timed("snapshot of 100 000 books", 4_000, || {
            bookx_store::health::take_snapshot(
                &o.store,
                &root,
                bookx_store::health::SnapshotKind::Manual,
                chrono::Utc::now(),
            )
            .unwrap()
        });
        timed("integrity check of 100 000 books", 5_000, || {
            assert!(o.store.check_integrity().is_empty())
        });
    }
    let o = timed("opening a 100 000 book library", 5_000, || {
        bookx_store::open_library(&root).unwrap()
    });
    assert_eq!(o.library.books.len(), N100K as usize);
    let found = timed("library search among 100 000", 500, || {
        o.library.search(
            "series:=\"Series 42\"",
            bookx_core::library::SortKey::Title,
            true,
        )
    });
    assert_eq!(found.len(), 34);
    let _ = std::fs::remove_dir_all(d);
}
