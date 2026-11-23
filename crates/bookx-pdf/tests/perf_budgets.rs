//! Performance budgets of the PDF engine. Same rules as `crates/bookx-core/tests/perf_budgets.rs`:
//! release budgets, debug gets 30x, `BOOKX_PERF_SCALE` scales them (`make perf` runs them in release).

use std::time::{Duration, Instant};

use bookx_pdf::testpdf::{OutlineEntry, Page, TestPdf};
use bookx_pdf::{Document, reflow};

fn budget(release_ms: u64) -> Duration {
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let f = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
    Duration::from_micros((release_ms as f64 * 1000.0 * f * scale) as u64)
}

fn check(what: &str, took: Duration, max: Duration) {
    assert!(took <= max, "{what}: took {took:?}, budget {max:?}");
}

fn book(pages: usize) -> Vec<u8> {
    let mut d = TestPdf::new().title("Perf");
    for i in 0..pages {
        let mut p = Page::new(612.0, 792.0).text(72.0, 740.0, 18.0, &format!("Section {}", i + 1));
        for j in 0..40 {
            p = p.text(
                72.0,
                700.0 - 16.0 * j as f32,
                11.0,
                &format!(
                    "Line {j} of page {i}: pack my box with five dozen liquor jugs and then some."
                ),
            );
        }
        d = d.page(p);
    }
    for k in 0..25 {
        d = d.outline(OutlineEntry::new(
            &format!("Part {k}"),
            k * (pages / 25).max(1),
        ));
    }
    d.build()
}

#[test]
fn opening_a_500_page_pdf_with_its_outline_and_labels() {
    let bytes = book(500);
    let t = Instant::now();
    let d = Document::open(bytes, None).unwrap();
    assert_eq!(d.page_count(), 500);
    let _ = d.page_sizes();
    let _ = d.outline();
    let _ = d.page_labels();
    let _ = d.info();
    check("open 500 pages", t.elapsed(), budget(400));
}

#[test]
fn rendering_one_page() {
    let d = Document::open(book(10), None).unwrap();
    let r = d.renderer();
    let _ = r.render(0, 1.5, false); // fonts and caches warm
    let t = Instant::now();
    for i in 1..6 {
        r.render(i, 1.5, false).unwrap();
    }
    check("5 pages at 1.5x", t.elapsed(), budget(400));
}

#[test]
fn extracting_the_text_of_a_page() {
    let d = Document::open(book(10), None).unwrap();
    let _ = d.text(0);
    let t = Instant::now();
    for i in 1..6 {
        assert!(d.text(i).lines.len() >= 40);
    }
    check("5 pages of text", t.elapsed(), budget(150));
}

#[test]
fn reflowing_a_book() {
    let d = Document::open(book(120), None).unwrap();
    let pages = d.all_text();
    let outline = d.outline();
    let t = Instant::now();
    let ch = reflow::reflow(&pages, &outline, "Perf");
    assert!(!ch.is_empty());
    check(
        "reflow 120 pages (after extraction)",
        t.elapsed(),
        budget(300),
    );
}

#[test]
fn searching_all_pages_of_cached_text() {
    let d = Document::open(book(200), None).unwrap();
    let pages = d.all_text();
    let t = Instant::now();
    let n: usize = pages.iter().map(|p| p.find("liquor jugs").len()).sum();
    assert_eq!(n, 200 * 40);
    check("search 200 pages", t.elapsed(), budget(300));
}
