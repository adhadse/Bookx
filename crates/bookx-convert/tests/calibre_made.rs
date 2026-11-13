//! Files written by Calibre's `ebook-convert` (version 9.15, from the synthetic `epub3-features`
//! corpus book) must read back completely: text, images, nested contents, links and style sheets.
//! The fixtures are committed so these checks run without Calibre; `build-aux/convert-parity.sh
//! --fixtures` regenerates them.

mod common;
mod corpus;

use bookx_convert::inspect::{score, Digest};
use bookx_convert::read_input;
use bookx_core::book::OpenBook;

fn fixture(name: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/calibre-made")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn read(name: &str) -> (OpenBook, Digest) {
    let book = read_input(&bookx_formats::default_readers(), &fixture(name), name)
        .unwrap_or_else(|e| panic!("reading {name}: {e:#}"));
    let d = Digest::of(&book);
    (book, d)
}

fn truth() -> Digest {
    let s = corpus::by_name("epub3-features");
    let book = read_input(&bookx_formats::default_readers(), &s.bytes, &s.file).unwrap();
    Digest::of(&book)
}

fn no_css_in_text(book: &OpenBook) {
    for p in book.spine_paths() {
        let text = book.archive.get_string(&p).unwrap_or_default();
        assert!(
            !text.contains("display: block") && !text.contains("margin: 0 5pt"),
            "style sheet text leaked into {p}"
        );
    }
}

#[test]
fn azw3_reads_text_links_styles_and_nested_contents() {
    let (book, d) = read("features.azw3");
    let t = truth();
    assert_eq!(d.title, t.title);
    assert!(d.authors.contains(&"ada lovelace".to_string()));
    assert!(d.has_cover);
    no_css_in_text(&book);
    let s = score(&t, &d);
    assert!(s.recall > 0.99, "recall {}", s.recall);
    assert_eq!(d.images, t.images);
    assert!(d.internal_links >= 7, "links: {}", d.internal_links);
    assert_eq!(d.broken_links, 0);
    let titles: Vec<(usize, &str)> = d
        .toc
        .iter()
        .map(|(depth, t)| (*depth, t.as_str()))
        .collect();
    assert_eq!(
        titles,
        vec![
            (0, "the first chapter"),
            (0, "the second chapter"),
            (1, "a section"),
            (0, "the third chapter"),
            (1, "deeper")
        ]
    );
    assert_eq!(d.toc_unresolved, 0);
    // the style sheet survives as CSS
    let css = book.archive.get_string("OEBPS/style.css").unwrap();
    assert!(css.contains("font-weight"), "{css}");
}

#[test]
fn mobi_reads_text_links_and_contents() {
    let (book, d) = read("features.mobi");
    let t = truth();
    assert_eq!(d.title, t.title);
    assert!(d.has_cover);
    no_css_in_text(&book);
    let s = score(&t, &d);
    assert!(s.recall > 0.99, "recall {}", s.recall);
    assert_eq!(d.images, t.images);
    assert_eq!(d.broken_links, 0);
    assert!(d.toc.len() >= 5, "{:?}", d.toc);
    assert!(d.toc.iter().any(|(_, t)| t == "deeper"));
    assert_eq!(d.toc_unresolved, 0);
}

#[test]
fn long_azw3_keeps_all_three_hundred_chapters_in_order() {
    let (_, d) = read("long-toc.azw3");
    assert_eq!(d.toc.len(), 300, "contents entries");
    assert_eq!(d.toc[0].1, "chapter 1");
    assert_eq!(d.toc[299].1, "chapter 300");
    assert_eq!(d.toc_unresolved, 0);
    let s = corpus::by_name("epub-long-toc");
    let truth =
        Digest::of(&read_input(&bookx_formats::default_readers(), &s.bytes, &s.file).unwrap());
    let sc = score(&truth, &d);
    assert!(sc.recall > 0.999 && sc.order > 0.999, "{sc:?}");
}

#[test]
fn rtl_and_cjk_text_survive_mobi_family() {
    for (file, src) in [("rtl.azw3", "epub-rtl"), ("cjk.mobi", "epub-ruby-cjk")] {
        let (_, d) = read(file);
        let s = corpus::by_name(src);
        let truth =
            Digest::of(&read_input(&bookx_formats::default_readers(), &s.bytes, &s.file).unwrap());
        let sc = score(&truth, &d);
        assert!(sc.recall > 0.98, "{file}: recall {}", sc.recall);
        assert!(sc.order > 0.98, "{file}: order {}", sc.order);
    }
}

#[test]
fn every_calibre_made_format_reads_with_its_text() {
    // Calibre's own PML/RTF/TXT writers glue list items and table cells into one word
    // ("alphaalpha one"), so those formats cannot reach the full recall.
    let t = truth();
    for (f, min_recall) in [
        ("features.pdb", 0.97),
        ("features.pmlz", 0.93),
        ("features.rb", 0.98),
        ("features.snb", 0.98),
        ("features.fb2", 0.999),
        ("features.docx", 0.999),
        ("features.rtf", 0.93),
        ("features.txtz", 0.97),
        ("features.htmlz", 0.999),
    ] {
        let (_, d) = read(f);
        let s = score(&t, &d);
        assert!(
            s.recall >= min_recall,
            "{f}: recall {} (extra {})",
            s.recall,
            s.extra
        );
        assert!(s.order > 0.95, "{f}: order {}", s.order);
    }
}

#[test]
fn rocket_ebook_keeps_accents_and_internal_links() {
    let (book, d) = read("features.rb");
    assert!(d.words.contains(&"café".to_string()), "accents lost");
    assert!(d.words.contains(&"crème".to_string()));
    assert_eq!(
        d.broken_links, 0,
        "anchors written as <a name=…> must stay targets"
    );
    assert!(book.cover_bytes().is_some() || d.images >= 1);
}
