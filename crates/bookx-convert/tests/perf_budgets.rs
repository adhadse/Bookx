//! Performance budgets for the conversion hot paths (see `AGENTS.md`). Budgets are for release
//! builds; debug builds get `DEBUG_FACTOR` times more room, `BOOKX_PERF_SCALE` scales them.

mod common;

use std::time::{Duration, Instant};

use bookx_convert::mobi::palmdoc;
use bookx_convert::mobi::{Azw3Writer, MobiWriter};
use bookx_convert::transforms::punctuation::smarten_html;
use bookx_convert::writers::{
    DocxWriter, EReaderWriter, EpubWriter, KepubWriter, OebWriter, PmlzWriter, RbWriter, RtfWriter,
    SnbWriter, ZipWriter,
};
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;

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

/// About 1.5 MB of text in 100 chapters.
fn big_book() -> OpenBook {
    let chapters: Vec<_> = (0..100)
        .map(|i| {
            let body: String = (0..60)
                .map(|j| format!("<p>Paragraph {i}.{j}: \"quoted\" text -- with enough words to look like a novel, ünïcödé and all. <a href=\"chapter-{}.xhtml#t\">next</a></p>\n", (i + 1) % 100 + 1))
                .collect();
            bookx_core::convert::Chapter {
                title: format!("Chapter {i}"),
                body: format!("<h1 id=\"t\">Chapter {i}</h1>\n{body}"),
            }
        })
        .collect();
    let arc = bookx_core::convert::create_epub(
        &common::metadata(),
        &chapters,
        Some((common::jpeg(300, 450), "jpg")),
    );
    OpenBook::from_archive(arc).unwrap()
}

#[test]
fn palmdoc_compresses_a_megabyte_quickly() {
    let text =
        "The quick brown fox jumps over the lazy dog, and \"quotes\" — ünïcödé. ".repeat(15_000);
    let t = Instant::now();
    let mut total = 0;
    for chunk in text.as_bytes().chunks(4096) {
        total += palmdoc::compress(chunk).len();
    }
    check("PalmDOC 1 MB", t.elapsed(), budget(250));
    assert!(total < text.len() / 2);
}

#[test]
fn mobi_and_azw3_of_a_large_book() {
    let book = big_book();
    let t = Instant::now();
    let mobi = MobiWriter::default().write(&book).unwrap();
    check("MOBI 100 chapters", t.elapsed(), budget(1500));
    let t = Instant::now();
    let azw = Azw3Writer.write(&book).unwrap();
    check("AZW3 100 chapters", t.elapsed(), budget(1500));
    assert!(mobi.len() > 100_000 && azw.len() > 100_000);
}

#[test]
fn epub_and_kepub_of_a_large_book() {
    let book = big_book();
    let t = Instant::now();
    EpubWriter::default().write(&book).unwrap();
    check("EPUB 100 chapters", t.elapsed(), budget(400));
    let t = Instant::now();
    KepubWriter::default().write(&book).unwrap();
    check("KEPUB 100 chapters", t.elapsed(), budget(1500));
}

#[test]
fn smartening_a_megabyte_of_markup() {
    let html: String = (0..8000)
        .map(|i| format!("<p>\"Line {i}\" -- it's 'quoted' and so is <i>this</i>...</p>\n"))
        .collect();
    let t = Instant::now();
    let out = smarten_html(&html);
    check("smarten 0.5 MB", t.elapsed(), budget(300));
    assert!(out.contains('“'));
}

fn timed(w: &dyn FormatWriter, book: &OpenBook, what: &str, release_ms: u64) -> Vec<u8> {
    let t = Instant::now();
    let out = w.write(book).unwrap();
    check(what, t.elapsed(), budget(release_ms));
    assert!(out.len() > 20_000, "{what}: {} bytes", out.len());
    out
}

#[test]
fn office_formats_of_a_large_book() {
    let book = big_book();
    timed(&DocxWriter::default(), &book, "DOCX 100 chapters", 150);
    timed(&RtfWriter, &book, "RTF 100 chapters", 150);
}

#[test]
fn palm_and_rocket_formats_of_a_large_book() {
    let book = big_book();
    timed(&EReaderWriter::default(), &book, "PDB 100 chapters", 150);
    timed(&PmlzWriter, &book, "PMLZ 100 chapters", 150);
    timed(&RbWriter, &book, "RB 100 chapters", 150);
}

#[test]
fn snb_of_a_large_book() {
    let book = big_book();
    timed(&SnbWriter, &book, "SNB 100 chapters", 500);
}

#[test]
fn zip_and_oeb_of_a_large_book() {
    let book = big_book();
    let t = Instant::now();
    let z = ZipWriter.write(&book).unwrap();
    let o = OebWriter.write(&book).unwrap();
    check("ZIP+OEB 100 chapters", t.elapsed(), budget(50));
    assert!(z.len() > 10_000 && o.len() > 10_000);
}

#[test]
fn structural_model_of_a_large_book() {
    let book = big_book();
    let t = Instant::now();
    let m = bookx_convert::model::Model::build(&book);
    check("model 100 chapters", t.elapsed(), budget(80));
    assert_eq!(m.sections.len(), 100);
}

#[test]
fn azw3_reader_with_fragments_and_contents_within_budget() {
    let book = big_book();
    let bytes = Azw3Writer.write(&book).unwrap();
    let t = Instant::now();
    let back =
        bookx_convert::read_input(&bookx_formats::default_readers(), &bytes, "big.azw3").unwrap();
    check(
        "read 1.5 MB AZW3 (KF8 tables, 100 chapters)",
        t.elapsed(),
        budget(1500),
    );
    assert!(back.toc().len() >= 100);
}

#[test]
fn legacy_encoding_detection_within_budget() {
    let text = "Все смешалось в доме Облонских. ".repeat(150_000);
    let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(&text);
    let t = Instant::now();
    let s = bookx_formats::common::decode_text(&bytes);
    check("decode 4.5 MB of cp1251", t.elapsed(), budget(400));
    assert!(s.starts_with("Все"));
}

#[test]
fn tag_soup_repair_within_budget() {
    let soup = format!(
        "<html><body>{}</body></html>",
        "<p>one <b>two <i>three</b> & four\n".repeat(20_000)
    );
    let t = Instant::now();
    let fixed = bookx_convert::markup::repair_to_xhtml(&soup).unwrap();
    check("repair 700 KB of tag soup", t.elapsed(), budget(1500));
    assert!(bookx_convert::markup::is_wellformed_xml(&fixed));
}
