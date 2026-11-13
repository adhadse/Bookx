//! The validator on books made by Bookx itself, on edited books and through the public API.

mod common;

use bookx_core::book::OpenBook;
use bookx_core::convert::{create_epub, Chapter};
use bookx_core::opf::Metadata;
use bookx_editing::validate::{
    render_json, render_text, summary_line, validate, validate_bytes, Severity,
};

fn made_book(chapters: usize) -> OpenBook {
    let md = Metadata {
        title: "Made by Bookx".into(),
        authors: vec!["A. Author".into()],
        language: "en".into(),
        ..Default::default()
    };
    let chapters: Vec<Chapter> = (0..chapters)
        .map(|i| Chapter {
            title: format!("Chapter {}", i + 1),
            body: format!(
                "<h1 id=\"h{i}\">Chapter {}</h1><p>Some text &amp; more.</p>\n",
                i + 1
            ),
        })
        .collect();
    OpenBook::from_archive(create_epub(
        &md,
        &chapters,
        Some((common::png(20, 30), "png")),
    ))
    .unwrap()
}

#[test]
fn books_created_by_bookx_have_no_errors() {
    let book = made_book(4);
    let r = validate(&book.archive);
    assert!(r.passed, "{}", render_text(&r));
    assert_eq!(r.summary.errors, 0);
    let bytes = book.archive.to_bytes().unwrap();
    let r2 = validate_bytes(&bytes);
    assert!(r2.passed, "{}", render_text(&r2));
    assert!(r2.messages.iter().all(|m| m.severity != Severity::Error));
}

#[test]
fn the_text_and_json_renderers_agree_with_the_report() {
    let mut book = made_book(2);
    book.archive.set("OEBPS/loose.txt", b"x".to_vec());
    let r = validate(&book.archive);
    assert!(r.has("OPF-003"));
    let text = render_text(&r);
    assert!(text.contains("OEBPS/loose.txt"));
    assert!(text.ends_with(&format!("{}\n", summary_line(&r))));
    let v: serde_json::Value = serde_json::from_str(&render_json(&r)).unwrap();
    assert_eq!(v["messages"].as_array().unwrap().len(), r.messages.len());
    assert_eq!(v["passed"].as_bool().unwrap(), r.passed);
}

#[test]
fn every_chapter_gets_checked() {
    let mut book = made_book(3);
    let path = book
        .package
        .manifest
        .iter()
        .filter(|i| i.media_type == "application/xhtml+xml" && i.href.contains("chapter-2"))
        .map(|i| format!("OEBPS/{}", i.href))
        .next()
        .expect("chapter 2");
    book.archive.set(&path, b"<html><body><p>unclosed".to_vec());
    let r = validate(&book.archive);
    let m = r.of("RSC-016");
    assert_eq!(m.len(), 1, "{}", render_text(&r));
    assert_eq!(m[0].file, path);
}
