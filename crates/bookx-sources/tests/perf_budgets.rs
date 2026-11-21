//! Performance budgets for bookx-sources. Budgets are for release builds; debug builds get
//! `DEBUG_FACTOR` times more room and `BOOKX_PERF_SCALE` scales everything.

use std::time::{Duration, Instant};

use bookx_core::library::Book;
use bookx_core::opf::Metadata;
use bookx_sources::{cover, dedup, feed, readability, template};
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

fn check<T>(what: &str, release_ms: u64, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let v = f();
    let took = t.elapsed();
    let max = budget(release_ms);
    assert!(took <= max, "{what}: took {took:?}, budget {max:?}");
    v
}

fn book(i: u64) -> Book {
    Book {
        id: i + 1,
        file: format!("{i}.epub"),
        cover: None,
        meta: Metadata {
            title: format!("Book number {i} of the grand collection"),
            authors: vec![format!("Firstname{} Lastname{}", i % 700, i % 997)],
            ..Default::default()
        },
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
    }
}

#[test]
fn duplicate_detection_in_a_large_library() {
    let mut books: Vec<Book> = (0..5000).map(book).collect();
    // plant 50 duplicates with different spellings
    for i in 0..50u64 {
        let mut d = book(i);
        d.id = 100_000 + i;
        d.meta.title = format!("The Book Number {i} of the Grand Collection!");
        books.push(d);
    }
    let sets = check("find_duplicates(5050 books)", 4000, || {
        dedup::find_duplicates(&books, dedup::DEFAULT_THRESHOLD)
    });
    assert_eq!(sets.len(), 50);
}

#[test]
fn template_rendering() {
    let t = template::Template::parse(
        "{author_sort:first_author}/{series?:}/{title:trunc(60)} ({year})",
    )
    .unwrap();
    let md = Metadata {
        title: "A Fairly Long Title With Several Words".into(),
        authors: vec!["Ada Lovelace".into()],
        published: "1843".into(),
        ..Default::default()
    };
    check("10k path renders", 300, || {
        for _ in 0..10_000 {
            let p = t.render_path(&md, "epub");
            assert!(!p.is_empty());
        }
    });
}

#[test]
fn feed_parsing_and_article_extraction() {
    let mut x = String::from("<rss version=\"2.0\"><channel><title>Big</title>");
    for i in 0..3000 {
        x.push_str(&format!("<item><title>Item {i}</title><link>https://x.org/{i}</link><pubDate>Tue, 05 Mar 2024 10:00:00 GMT</pubDate><description>Some &lt;b&gt;text&lt;/b&gt; {i}</description></item>"));
    }
    x.push_str("</channel></rss>");
    let f = check("parse 3000-item RSS", 800, || {
        feed::parse_feed(x.as_bytes()).unwrap()
    });
    assert_eq!(f.entries.len(), 3000);

    let mut html = String::from("<html><head><title>Big</title></head><body>");
    for i in 0..3000 {
        html.push_str(&format!("<div class=\"row\"><p>Paragraph {i}, with filler words, commas, and more filler to reach a sensible length.</p></div>"));
    }
    html.push_str("</body></html>");
    let a = check("extract a 400 KB page", 1500, || {
        readability::extract(&html, "https://x.org/").unwrap()
    });
    assert!(a.text_len > 100_000);
}

#[test]
fn cover_generation_and_processing() {
    let png = check("generate a 600x900 cover", 300, || {
        cover::generate_cover(
            "A Title That Wraps Onto Several Lines",
            "Some Author",
            "A Series",
            600,
            900,
        )
        .unwrap()
    });
    check("resize + dominant colour + blank check", 600, || {
        let small = cover::resize_max(&png, 200, 300).unwrap();
        cover::dominant_color(&small).unwrap();
        assert!(!cover::is_blank(&png).unwrap());
    });
}

#[test]
fn name_tidying_and_series_overview_at_100k_books() {
    let books: Vec<Book> = (0..100_000u64)
        .map(|i| {
            let mut b = book(i);
            // a third of the authors are written "Last, First"
            if i % 3 == 0 {
                b.meta.authors = vec![format!("Lastname{}, Firstname{}", i % 997, i % 700)];
            }
            b.meta.series = format!("Series {}", i % 3000);
            b.meta.series_index = (i % 12) as f64;
            b
        })
        .collect();
    let plan = check("tidy plan for 100 000 books", 3000, || {
        bookx_sources::tidy::plan(&books)
    });
    assert!(!plan.is_empty());
    let groups = check("series overview of 100 000 books", 1500, || {
        bookx_sources::tidy::series_overview(&books)
    });
    assert_eq!(groups.len(), 3000);
}

#[test]
fn weak_metadata_scan_and_identifier_cleanup_at_100k_books() {
    let books: Vec<Book> = (0..100_000u64)
        .map(|i| {
            let mut b = book(i);
            if i % 5 == 0 {
                b.meta.title = format!("scan_{i}");
            }
            if i % 7 == 0 {
                b.meta.identifiers = vec![bookx_core::opf::Identifier {
                    scheme: "ISBN".into(),
                    value: "978-0-441-17271-9".into(),
                    xml_id: None,
                }];
            }
            b
        })
        .collect();
    let weak = check("weak:-scan of 100 000 books", 300, || {
        books
            .iter()
            .filter(|b| !bookx_core::quality::weak_reasons(b).is_empty())
            .count()
    });
    assert_eq!(weak, 100_000, "no cover and no language on every book");
    let fixes = check(
        "identifier cleanup suggestions for 100 000 books",
        1500,
        || {
            books
                .iter()
                .filter(|b| !bookx_sources::idclean::suggest(&b.meta).is_empty())
                .count()
        },
    );
    assert_eq!(
        fixes,
        books
            .iter()
            .filter(|b| !b.meta.identifiers.is_empty())
            .count()
    );
}

#[test]
fn grouping_authors_in_a_large_library() {
    let books: Vec<Book> = (0..20_000).map(book).collect();
    let map = bookx_sources::aliases::AliasMap::from_pairs(&[(
        "Firstname1 Lastname1".to_string(),
        "Firstname2 Lastname2".to_string(),
    )]);
    let groups = check("group_authors(20000 books)", 1500, || {
        bookx_sources::aliases::group_authors(&books, &map)
    });
    assert!(groups.len() > 500);
    let plan = check("plan_rename(20000 books)", 1000, || {
        bookx_sources::aliases::plan_rename(
            &books,
            &["Firstname5 Lastname5".to_string()],
            "Renamed Person",
        )
    });
    assert!(!plan.items.is_empty());
}

#[test]
fn matching_a_tracker_export_against_a_large_library() {
    let books: Vec<Book> = (0..10_000).map(book).collect();
    let mut csv = String::from("Title,Author,My Rating,Exclusive Shelf\n");
    for i in (0..2000u64).step_by(2) {
        csv.push_str(&format!(
            "Book number {i} of the grand collection,Firstname{} Lastname{},4,read\n",
            i % 700,
            i % 997
        ));
    }
    let rows = bookx_sources::trackers::parse(&csv).unwrap();
    let m = check("match_rows(1000 rows x 10000 books)", 3000, || {
        bookx_sources::trackers::match_rows(&rows, &books)
    });
    assert_eq!(m.iter().filter(|x| x.book.is_some()).count(), rows.len());
}

#[test]
fn audiobook_pairing_in_a_large_library() {
    use bookx_sources::audiobooks::{pair, Candidate};
    let books: Vec<Book> = (0..20_000).map(book).collect();
    let cands: Vec<Candidate> = (0..3000u64)
        .map(|i| Candidate {
            path: format!("/audio/{i}/Book number {i} of the grand collection.m4b").into(),
            title_guess: format!("Book number {} of the grand collection", i * 3),
            author_guess: String::new(),
            isbn: None,
            files: 1,
            bytes: 1,
        })
        .collect();
    let pairs = check("audiobooks::pair(20 000 books x 3 000 files)", 1500, || {
        pair(&books, &cands, &|_| None)
    });
    assert_eq!(pairs.len(), 3000);
}

// -- duplicate scan at 100 000 books (PP-C-11, PP-G-12) -----------------------------------------------

#[path = "../../bookx-store/tests/common/synth.rs"]
mod synth;

#[test]
fn duplicate_scan_of_a_hundred_thousand_books() {
    let books = synth::synth_library(100_000);
    let sets = check("find_duplicates(100 000 books)", 6_000, || {
        dedup::find_duplicates(&books, dedup::DEFAULT_THRESHOLD)
    });
    // every 500th book is a planted near-duplicate of the one before it
    assert!(sets.len() >= 190, "found {} sets", sets.len());
}
