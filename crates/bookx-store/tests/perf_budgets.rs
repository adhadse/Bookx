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

#[test]
fn loading_ten_thousand_records_keeps_the_order_on_several_threads() {
    let s = SqliteStore::open_in_memory().unwrap();
    let books: Vec<_> = (1..=10_000u64)
        .map(|id| book(id, &format!("Title {id}"), &format!("Author {}", id % 50)))
        .collect();
    s.put_many(&books).unwrap();
    let all = timed("load 10 000 records", 300, || s.all_books().unwrap());
    assert_eq!(all.len(), 10_000);
    assert!(all.windows(2).all(|w| w[0].id < w[1].id));
    assert_eq!(all[4_321].meta.title, "Title 4322");
    assert!(
        all.iter()
            .zip(&books)
            .all(|(a, b)| a.id == b.id && a.meta.title == b.meta.title)
    );
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

// -- 100 000 books (roadmap PP-C-11, PP-G-12) -----------------------------------------------------
//
// A synthetic library (`tests/common/synth.rs`: reproducible, no real content) of the size the roadmap
// asks for. Budgets are for release builds and sit about 2 to 5 times above what an ordinary desktop
// measures (numbers in docs/performance.md); the point is to catch an accidental O(n^2), not to race.
// `cargo run --release -p bookx-store --example scale -- 10000 50000 100000` prints the full table.

#[path = "common/synth.rs"]
mod synth;

const N100K: u64 = 100_000;

#[test]
fn a_hundred_thousand_book_library() {
    use bookx_core::library::{Category, SortKey};

    let d = lib_tmp("100k");
    let root = d.join("lib");
    {
        let mut o = bookx_store::open_library(&root).unwrap();
        o.library.books = synth::synth_library(N100K);
        o.library.books[77_777].meta.title = "Zanzibarlike Needle".into();
        timed("first save of 100 000 books", 25_000, || {
            o.library.save().unwrap()
        });
        assert_eq!(o.store.len(), N100K as usize);

        timed("one progress save among 100 000 books", 500, || {
            o.library.books[54_321].progress = 0.5;
            o.library.save().unwrap()
        });
        timed("saving 100 000 unchanged books", 500, || {
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
        let common = timed("metadata search, a word in half the books", 1_500, || {
            o.store.search_ids("tal")
        });
        assert!(common.len() > 10_000);
        timed("snapshot of 100 000 books", 4_000, || {
            bookx_store::health::take_snapshot(
                &o.store,
                &root,
                bookx_store::health::SnapshotKind::Manual,
                chrono::Utc::now(),
            )
            .unwrap()
        });
        let bak = d.join("backup.sqlite");
        timed("online backup of 100 000 books", 1_500, || {
            o.store.backup_to(&bak).unwrap()
        });
        assert!(std::fs::metadata(&bak).unwrap().len() > 100_000_000);
        timed("integrity check of 100 000 books", 5_000, || {
            assert!(o.store.check_integrity().is_empty())
        });
    }
    let rss_before = synth::rss_kb();
    let o = timed("opening a 100 000 book library", 500, || {
        bookx_store::open_library(&root).unwrap()
    });
    assert_eq!(o.library.books.len(), N100K as usize);
    if rss_before > 0 {
        // the whole catalogue lives in memory: about 1.7 kB per book (measured 180 MB at 100 000)
        let grown = synth::rss_kb().saturating_sub(rss_before);
        assert!(
            grown < 450_000,
            "opening 100 000 books grew RSS by {grown} kB"
        );
    }
    let lib = &o.library;
    let found = timed("series filter among 100 000", 300, || {
        lib.search(
            &format!("series:=\"{}\"", lib.books[2].meta.series),
            SortKey::Title,
            true,
        )
    });
    assert!(!found.is_empty() || lib.books[2].meta.series.is_empty());
    for (name, key, ms) in [
        ("title", SortKey::Title, 400),
        ("author", SortKey::Author, 400),
        ("date added", SortKey::DateAdded, 300),
        ("series", SortKey::Series, 800),
    ] {
        let ids = timed(&format!("list sorted by {name}, 100 000 books"), ms, || {
            lib.search("", key, true)
        });
        assert_eq!(ids.len(), N100K as usize);
    }
    for (name, q) in [
        ("tag filter", "tags:tag-17"),
        ("rating filter", "rating:>=8"),
        ("free text", "kobaltal"),
        ("language and tag", "language:de tags:tag-3"),
    ] {
        timed(&format!("{name} among 100 000"), 400, || {
            lib.search(q, SortKey::Title, true)
        });
    }
    let all = lib.search("", SortKey::Title, true);
    // what the grid does with the ids: this was a `get` per id, 3 s at 100 000 books
    let shown = timed("books for 100 000 visible ids", 150, || lib.books_for(&all));
    assert_eq!(shown.len(), N100K as usize);
    timed("sidebar categories of 100 000 books", 500, || {
        (
            lib.categories(Category::Authors).len(),
            lib.categories(Category::Series).len(),
            lib.categories(Category::Tags).len(),
        )
    });
    let _ = std::fs::remove_dir_all(d);
}
