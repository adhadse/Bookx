//! KEPUB output validated with `zip` + `roxmltree`.

mod common;

use std::collections::HashSet;
use std::io::{Cursor, Read};

use bookx_convert::writers::KepubWriter;
use bookx_core::contracts::FormatWriter;
use roxmltree::Document;
use zip::ZipArchive;

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut b = Vec::new();
            f.read_to_end(&mut b).unwrap();
            (f.name().unwrap().to_string(), b)
        })
        .collect()
}

fn doc(text: &str) -> Document<'_> {
    Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap()
}

fn text_without_spans(doc: &Document) -> String {
    doc.descendants()
        .find(|n| n.has_tag_name("body"))
        .unwrap()
        .descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>()
}

#[test]
fn writer_identity() {
    let w = KepubWriter::default();
    assert_eq!(w.id(), "kepub");
    assert_eq!(w.extension(), "kepub.epub");
}

#[test]
fn spans_wrappers_and_unchanged_text() {
    let book = common::sample_book();
    let bytes = KepubWriter::default().write(&book).unwrap();
    let files = unzip(&bytes);
    assert_eq!(files[0].0, "mimetype");
    let by_name: std::collections::HashMap<&str, &Vec<u8>> =
        files.iter().map(|(n, b)| (n.as_str(), b)).collect();

    for (i, path) in book.spine_paths().iter().enumerate() {
        let original = book.archive.get_string(path).unwrap();
        let after = String::from_utf8(by_name[path.as_str()].clone()).unwrap();
        let d0 = doc(&original);
        let d1 = doc(&after);

        // text is identical once the spans are looked through
        assert_eq!(
            text_without_spans(&d0),
            text_without_spans(&d1),
            "chapter {i}"
        );

        let body = d1.descendants().find(|n| n.has_tag_name("body")).unwrap();
        let elems: Vec<_> = body.children().filter(|n| n.is_element()).collect();
        assert_eq!(elems.len(), 1, "body holds only the wrapper");
        assert_eq!(elems[0].attribute("id"), Some("book-columns"));
        let inner: Vec<_> = elems[0].children().filter(|n| n.is_element()).collect();
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].attribute("id"), Some("book-inner"));

        // spans: class, unique ids of the form kobo.P.S, never nested, never empty
        let mut seen = HashSet::new();
        let spans: Vec<_> = d1
            .descendants()
            .filter(|n| n.has_tag_name("span") && n.attribute("class") == Some("koboSpan"))
            .collect();
        assert!(!spans.is_empty());
        for s in &spans {
            let id = s.attribute("id").unwrap();
            let parts: Vec<&str> = id.split('.').collect();
            assert_eq!(parts.len(), 3, "{id}");
            assert_eq!(parts[0], "kobo");
            assert!(
                parts[1].parse::<u32>().is_ok() && parts[2].parse::<u32>().is_ok(),
                "{id}"
            );
            assert!(seen.insert(id.to_string()), "duplicate {id}");
            assert!(!s
                .descendants()
                .skip(1)
                .any(|d| d.has_tag_name("span") && d.attribute("class") == Some("koboSpan")));
            assert!(
                !s.text().unwrap_or("x").trim().is_empty() || s.children().any(|c| c.is_element())
            );
        }
        // every non-blank text node of the body is inside a span
        for t in body.descendants().filter(|n| n.is_text()) {
            if t.text().unwrap().trim().is_empty() || t.ancestors().any(|a| a.has_tag_name("pre")) {
                continue; // preformatted text is deliberately left alone
            }
            assert!(
                t.ancestors()
                    .any(|a| a.has_tag_name("span") && a.attribute("class") == Some("koboSpan")),
                "unwrapped text {:?}",
                t.text()
            );
        }
    }
}

#[test]
fn sentences_get_their_own_spans() {
    let book = common::sample_book();
    let bytes = KepubWriter::default().write(&book).unwrap();
    let files = unzip(&bytes);
    let ch1 = files
        .iter()
        .find(|(n, _)| n.ends_with("chapter-1.xhtml"))
        .unwrap();
    let text = String::from_utf8(ch1.1.clone()).unwrap();
    let d = doc(&text);
    let first_p = d.descendants().find(|n| n.has_tag_name("p")).unwrap();
    let spans: Vec<String> = first_p
        .descendants()
        .filter(|n| n.has_tag_name("span"))
        .filter_map(|n| n.text().map(|t| t.to_string()))
        .collect();
    assert!(spans.len() >= 2, "{spans:?}");
    assert!(spans[0].ends_with("rain fell. "), "{spans:?}");
}

#[test]
fn package_is_still_a_valid_epub() {
    let book = common::sample_book();
    let bytes = KepubWriter::default().write(&book).unwrap();
    let files = unzip(&bytes);
    assert_eq!(files[0].0, "mimetype");
    let container = String::from_utf8(
        files
            .iter()
            .find(|(n, _)| n == "META-INF/container.xml")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    let c = doc(&container);
    let opf_path = c
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .unwrap()
        .attribute("full-path")
        .unwrap();
    let opf =
        String::from_utf8(files.iter().find(|(n, _)| n == opf_path).unwrap().1.clone()).unwrap();
    let o = doc(&opf);
    assert_eq!(
        o.root_element().attribute("version"),
        Some("2.0"),
        "KEPUB defaults to EPUB 2 packaging"
    );
    // images and cover survive byte for byte
    let cover = book.cover_bytes().unwrap();
    assert!(files.iter().any(|(_, b)| b == cover));
}

#[test]
fn rewriting_a_kepub_does_not_double_wrap() {
    let book = common::sample_book();
    let once = KepubWriter::default().write(&book).unwrap();
    let again_book = bookx_core::book::OpenBook::from_archive(
        bookx_core::archive::Archive::from_bytes(once.clone()).unwrap(),
    )
    .unwrap();
    let twice = KepubWriter::default().write(&again_book).unwrap();
    let a = unzip(&once);
    let b = unzip(&twice);
    for (n, bytes) in &a {
        if n.ends_with("chapter-1.xhtml") {
            let other = &b.iter().find(|(m, _)| m == n).unwrap().1;
            assert_eq!(bytes, other);
        }
    }
}
