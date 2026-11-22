//! The conversion report: what an output format cannot hold is named, and the table of formats
//! only claims what the writers really keep.

mod common;

use std::sync::atomic::AtomicBool;

use bookx_convert::inspect::Digest;
use bookx_convert::report::{build_report, caps};
use bookx_convert::{ConvertOptions, convert, read_input};

fn go(ext: &str) -> bookx_convert::Converted {
    let epub = common::nested_toc_book().archive.to_bytes().unwrap();
    convert(
        &bookx_formats::default_readers(),
        &epub,
        "b.epub",
        ext,
        &ConvertOptions::default(),
        &|_| {},
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn plain_text_loses_pictures_cover_and_contents() {
    let r = go("txt").report;
    let all = r.lost.join(" | ");
    assert!(
        all.contains("images") && all.contains("cover") && all.contains("table of contents"),
        "{all}"
    );
    assert!(r.to_text().contains("lost:"));
}

#[test]
fn epub_loses_nothing_and_says_so() {
    let r = go("epub").report;
    assert!(r.lost.is_empty(), "{:?}", r.lost);
    assert!(r.to_text().contains("Nothing was lost"));
    assert!(r.words > 20 && r.toc_entries == 4);
}

#[test]
fn transform_notes_are_listed() {
    let book = common::sample_book();
    let rep = bookx_core::contracts::TransformReport {
        files_changed: 2,
        notes: vec!["did a thing".into()],
    };
    let r = build_report(&book, "epub", &[("smarten".into(), rep)]);
    assert!(r.changes.iter().any(|c| c.contains("did a thing")));
}

/// Whatever the table says a format keeps must survive a write/read cycle.
#[test]
fn capability_table_matches_what_writers_keep() {
    let readers = bookx_formats::default_readers();
    let truth = Digest::of(&common::nested_toc_book());
    assert!(truth.images >= 1 && truth.has_cover && truth.toc.len() == 4);
    for ext in [
        "epub", "kepub", "mobi", "azw3", "fb2", "htmlz", "txtz", "txt", "html", "md", "docx",
        "rtf", "pdb", "pmlz", "rb", "snb",
    ] {
        let out = go(ext);
        let back = read_input(&readers, &out.bytes, &format!("x.{}", out.extension)).unwrap();
        let d = Digest::of(&back);
        let c = caps(ext);
        if c.images {
            assert!(d.images >= 1, "{ext} claims to keep images");
        }
        if c.cover {
            assert!(d.has_cover, "{ext} claims to keep the cover");
        }
        if c.toc {
            assert!(!d.toc.is_empty(), "{ext} claims to keep the contents");
        }
        if c.nested_toc {
            assert!(
                d.toc.iter().any(|t| t.0 > 0),
                "{ext} claims nested contents: {:?}",
                d.toc
            );
        }
    }
}
