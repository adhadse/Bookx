//! CSS transforms, style filtering, compare and reports on real EPUB content.

mod common;

use bookx_editing::compare::{compare_books, html_diff, CompareOptions};
use bookx_editing::filter_styles::{filter_styles, list_properties, FilterSpec};
use bookx_editing::reports::{build_report, ReportOptions};
use bookx_editing::transform_css::{transform_book, CssTransform, Unit};

fn styled() -> bookx_core::book::OpenBook {
    let mut b = common::book_from_bodies(&[
        "<h1>Title</h1><p class=\"a\" style=\"color: RED; -webkit-hyphens: auto\">one</p><p class=\"b\">two</p>".to_string(),
        "<p class=\"a\">three</p>".to_string(),
    ]);
    b.archive.set_string(
        "OEBPS/style.css",
        "body { font-family: \"Times New Roman\", serif; font-size: 12pt }\n.a { color: #f00; margin: 0 0 6pt 0; -webkit-hyphens: auto }\n.b { color: rgb(0, 0, 255) }\n.unused { color: red }\n",
    );
    b
}

#[test]
fn transform_then_compare_then_report() {
    let mut b = styled();
    let original = b.archive.clone();
    let rep = transform_book(
        &mut b,
        &[
            CssTransform::ReplaceColor {
                from: "red".into(),
                to: "#cc0000".into(),
            },
            CssTransform::ConvertUnits {
                from: Unit::Pt,
                to: Unit::Px,
                base_px: 16.0,
                properties: vec![],
            },
            CssTransform::RenameFontFamily {
                from: "Times New Roman".into(),
                to: "Georgia".into(),
            },
        ],
    );
    assert_eq!(rep.total_changes, 6, "{rep:?}");
    let css = b.archive.get_string("OEBPS/style.css").unwrap();
    assert!(css.contains("font-family: \"Georgia\", serif; font-size: 16px"));
    assert!(css.contains(".a { color: #cc0000; margin: 0 0 8px 0;"));
    assert!(css.contains(".unused { color: #cc0000 }"));
    assert!(css.contains("rgb(0, 0, 255)"), "other colours untouched");
    assert!(b
        .archive
        .get_string("OEBPS/chapter-1.xhtml")
        .unwrap()
        .contains("style=\"color: #cc0000;"));
    common::validate_epub(&b.archive.to_bytes().unwrap());

    let d = compare_books(
        &common::reopen(&bookx_core::book::OpenBook::from_archive(original).unwrap()),
        &b,
        CompareOptions::default(),
    );
    let changed: Vec<&str> = d.changed.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(changed, vec!["OEBPS/chapter-1.xhtml", "OEBPS/style.css"]);
    assert!(d.added.is_empty() && d.removed.is_empty());
    assert!(
        html_diff("font-size: 12pt", "font-size: 16px").contains("<del>12pt</del><ins>16px</ins>")
    );

    let r = build_report(
        &b,
        &ReportOptions {
            top_words: 3,
            top_chars: 3,
        },
    );
    let unused: Vec<&str> = r.css.files[0]
        .rules
        .iter()
        .filter(|x| !x.used)
        .map(|x| x.selector.as_str())
        .collect();
    assert_eq!(unused, vec![".unused"]);
    assert_eq!(r.words.overall.top.len(), 3);
}

#[test]
fn filter_styles_and_inventory() {
    let mut b = styled();
    assert_eq!(list_properties(&b)["-webkit-hyphens"], 2);
    let r = filter_styles(
        &mut b,
        &FilterSpec {
            properties: vec!["-webkit-*".into(), "margin".into()],
            remove_empty_rules: true,
            remove_empty_style_attrs: true,
        },
    );
    assert_eq!(r.declarations_removed, 3);
    assert!(!b
        .archive
        .get_string("OEBPS/style.css")
        .unwrap()
        .contains("-webkit"));
    assert!(!b
        .archive
        .get_string("OEBPS/chapter-1.xhtml")
        .unwrap()
        .contains("-webkit"));
    common::validate_epub(&b.archive.to_bytes().unwrap());
    assert!(list_properties(&b)
        .keys()
        .all(|k| !k.starts_with("-webkit") && k != "margin"));
}
