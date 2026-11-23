//! Performance budgets for the input readers of the second wave (see `AGENTS.md`). Budgets are
//! for release builds; debug builds get `DEBUG_FACTOR` times more room, `BOOKX_PERF_SCALE`
//! scales them.

mod common;

use std::time::{Duration, Instant};

use bookx_formats::read_any;
use common::fmt_writers::*;

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

fn check(what: &str, took: Duration, max: Duration) {
    assert!(
        took <= max,
        "{what}: took {took:?}, budget {max:?} (BOOKX_PERF_SCALE adjusts budgets)"
    );
}

/// About 1.5 MB of plain prose in `chapters` chapters.
fn prose(chapters: usize) -> String {
    let line =
        "The quick brown fox jumps over the lazy dog, again and again, until the story ends. ";
    (0..chapters)
        .map(|i| format!("Chapter {i}\n\n{}\n\n", line.repeat(60)))
        .collect()
}

#[test]
fn ereader_pml_book_converts_within_budget() {
    let text: String = (0..200)
        .map(|i| {
            format!(
                "\\x\\cChapter {i}\\c\\x\n{}",
                "A paragraph with \\iitalic\\i and \\bbold\\b text, caf\\a233, and more words.\n"
                    .repeat(150)
            )
        })
        .collect();
    let f = write_ereader(&EReaderSpec {
        version: 10,
        title: "Perf".into(),
        text,
        ..Default::default()
    });
    let t = Instant::now();
    let arc = read_any("p.pdb", &f).unwrap();
    let took = t.elapsed();
    check("eReader 1.5 MB PML", took, budget(300));
    assert!(arc.to_bytes().unwrap().len() > 10_000);
}

#[test]
fn chm_lzx_book_converts_within_budget() {
    let files: Vec<(String, Vec<u8>)> = (0..60)
        .map(|i| {
            (
                format!("/p{i}.htm"),
                format!(
                    "<html><body><h1>Page {i}</h1>{}</body></html>",
                    "<p>The quick brown fox jumps over the lazy dog, again and again.</p>"
                        .repeat(400)
                )
                .into_bytes(),
            )
        })
        .collect();
    let f = write_chm(&ChmSpec {
        title: "Perf".into(),
        files,
        compressed: true,
        reset_frames: 2,
        ..Default::default()
    });
    let t = Instant::now();
    let arc = read_any("p.chm", &f).unwrap();
    let took = t.elapsed();
    check("CHM 1.6 MB LZX", took, budget(250));
    assert!(arc.to_bytes().unwrap().len() > 10_000);
}

#[test]
fn snb_book_converts_within_budget() {
    let body: String = (0..6000)
        .map(|i| {
            format!("<text>Paragraph {i}: the quick brown fox jumps over the lazy dog.</text>")
        })
        .collect();
    let ch = format!("<snbc><head><name>Big</name></head><body>{body}</body></snbc>");
    let f = write_snb(&[
        (
            "book.snbf",
            b"<book-snbf><head><name>Perf</name></head></book-snbf>".to_vec(),
            true,
        ),
        ("ch.snbc", ch.into_bytes(), true),
    ]);
    let t = Instant::now();
    read_any("p.snb", &f).unwrap();
    let took = t.elapsed();
    check("SNB 0.4 MB bzip2", took, budget(100));
}

#[test]
fn tcr_book_converts_within_budget() {
    let f = tcr_compress(prose(300).as_bytes());
    let t = Instant::now();
    read_any("p.tcr", &f).unwrap();
    let took = t.elapsed();
    check("TCR 1.5 MB", took, budget(150));
}

#[test]
fn rb_book_converts_within_budget() {
    let html: String = (0..300)
        .map(|i| {
            format!(
                "<h1>Chapter {i}</h1>{}",
                "<p>The quick brown fox jumps over the lazy dog, again and again.</p>".repeat(60)
            )
        })
        .collect();
    let f = write_rb(&RbSpec {
        info: "TITLE=Perf\n".into(),
        pages: vec![("1.html".into(), html.into_bytes(), true)],
        ..Default::default()
    });
    let t = Instant::now();
    read_any("p.rb", &f).unwrap();
    let took = t.elapsed();
    check("RB 1.2 MB", took, budget(150));
}

#[test]
fn rar_listing_and_stream_cutting_of_the_largest_accepted_archive_stay_fast() {
    use bookx_formats::rar::{
        MAX_LISTED, check_listing, parse_7z_list, parse_unrar_list, split_stream,
    };
    let mut seven = String::new();
    let mut unrar = String::new();
    for i in 0..MAX_LISTED {
        seven +=
            &format!("Path = dir/page-{i:05}.jpg\nSize = 4096\nFolder = -\nAttributes = A\n\n");
        unrar += &format!(
            "        Name: dir/page-{i:05}.jpg\n        Type: File\n        Size: 4096\n\n"
        );
    }
    let t = Instant::now();
    let listed = parse_7z_list(&seven);
    let listed2 = parse_unrar_list(&unrar);
    assert_eq!(listed.len(), MAX_LISTED);
    assert_eq!(listed2.len(), MAX_LISTED);
    check_listing(&listed).unwrap();
    check("two listings of 20,000 entries", t.elapsed(), budget(400));
    let stream = vec![0u8; MAX_LISTED * 4096];
    let t = Instant::now();
    let files = split_stream(&listed, &stream).unwrap();
    assert_eq!(files.len(), MAX_LISTED);
    check("cutting 80 MB into 20,000 pages", t.elapsed(), budget(400));
}
