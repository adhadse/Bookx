//! DOCX writer: package structure validated with an independent XML parser, relationships and
//! content types resolved by hand, the formats reader as a round-trip oracle and Calibre's DOCX
//! reader as an external one.

mod common;

use std::collections::{HashMap, HashSet};

use bookx_convert::writers::{DocxOptions, DocxWriter, PageSize};
use bookx_core::contracts::FormatWriter;
use common::*;
use roxmltree::Document;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn write(book: &bookx_core::book::OpenBook) -> Vec<(String, Vec<u8>)> {
    unzip(&DocxWriter::default().write(book).unwrap())
}

fn write_with(book: &bookx_core::book::OpenBook, options: DocxOptions) -> Vec<(String, Vec<u8>)> {
    unzip(&DocxWriter { options }.write(book).unwrap())
}

fn doc_xml(files: &[(String, Vec<u8>)]) -> String {
    part_str(files, "word/document.xml")
}

fn w_attr<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.attribute((W, name))
}

fn count(doc: &Document, name: &str) -> usize {
    doc.descendants()
        .filter(|n| n.is_element() && n.has_tag_name((W, name)))
        .count()
}

/// Plain text of the body the way Word shows it: `w:t` runs, tabs and breaks.
fn body_text(doc: &Document) -> String {
    let mut out = String::new();
    for p in doc
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name((W, "p")))
    {
        for n in p.descendants() {
            if n.is_element() && n.has_tag_name((W, "t")) {
                out.push_str(n.text().unwrap_or(""));
            } else if n.is_element() && n.has_tag_name((W, "tab")) {
                out.push('\t');
            }
        }
        out.push('\n');
    }
    out
}

#[test]
fn every_part_is_well_formed_xml_and_the_package_is_complete() {
    let files = write(&sample_book());
    for name in [
        "[Content_Types].xml",
        "_rels/.rels",
        "docProps/core.xml",
        "docProps/app.xml",
        "word/document.xml",
        "word/styles.xml",
        "word/numbering.xml",
        "word/settings.xml",
        "word/_rels/document.xml.rels",
    ] {
        let text = part_str(&files, name);
        assert!(text.starts_with("<?xml"), "{name} lacks an XML declaration");
        Document::parse(&text).unwrap_or_else(|e| panic!("{name} is not well-formed: {e}"));
    }
    let names: HashSet<_> = files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(names.len(), files.len(), "duplicate zip entries");
}

#[test]
fn content_types_cover_every_part() {
    let files = write(&sample_book());
    let ct = part_str(&files, "[Content_Types].xml");
    let doc = Document::parse(&ct).unwrap();
    let mut defaults = HashMap::new();
    let mut overrides = HashMap::new();
    for n in doc.root_element().children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "Default" => {
                defaults.insert(
                    n.attribute("Extension").unwrap().to_ascii_lowercase(),
                    n.attribute("ContentType").unwrap(),
                );
            }
            "Override" => {
                overrides.insert(
                    n.attribute("PartName").unwrap(),
                    n.attribute("ContentType").unwrap(),
                );
            }
            other => panic!("unexpected {other}"),
        }
    }
    for (name, _) in &files {
        if name == "[Content_Types].xml" {
            continue;
        }
        let ext = name.rsplit('.').next().unwrap().to_ascii_lowercase();
        assert!(
            overrides.contains_key(format!("/{name}").as_str()) || defaults.contains_key(&ext),
            "no content type for {name}"
        );
    }
    for part in overrides.keys() {
        assert!(
            files.iter().any(|(n, _)| format!("/{n}") == *part),
            "override for missing part {part}"
        );
    }
    assert_eq!(
        overrides["/word/document.xml"],
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
    );
    assert_eq!(defaults["png"], "image/png");
    assert_eq!(defaults["jpeg"], "image/jpeg");
}

#[test]
fn relationships_resolve_and_ids_are_unique() {
    let files = write(&sample_book());
    // package relationships
    let root = part_str(&files, "_rels/.rels");
    let rd = Document::parse(&root).unwrap();
    let mut types = Vec::new();
    for r in rd.descendants().filter(|n| n.has_tag_name("Relationship")) {
        let target = r.attribute("Target").unwrap();
        assert!(
            files.iter().any(|(n, _)| n == target),
            "dangling root rel {target}"
        );
        types.push(
            r.attribute("Type")
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string(),
        );
    }
    types.sort();
    assert_eq!(
        types,
        ["core-properties", "extended-properties", "officeDocument"]
    );

    // document relationships
    let rels = part_str(&files, "word/_rels/document.xml.rels");
    let rd = Document::parse(&rels).unwrap();
    let mut ids = HashSet::new();
    let mut external = 0;
    for r in rd.descendants().filter(|n| n.has_tag_name("Relationship")) {
        let id = r.attribute("Id").unwrap();
        assert!(ids.insert(id.to_string()), "duplicate relationship id {id}");
        let target = r.attribute("Target").unwrap();
        if r.attribute("TargetMode") == Some("External") {
            external += 1;
            assert!(r.attribute("Type").unwrap().ends_with("/hyperlink"));
            assert!(target.starts_with("http"));
        } else {
            assert!(
                files.iter().any(|(n, _)| *n == format!("word/{target}")),
                "dangling relationship target {target}"
            );
        }
    }
    assert_eq!(external, 1, "one external link in the sample book");

    // every r:id / r:embed used by the document is declared
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    for n in doc.descendants().filter(|n| n.is_element()) {
        for a in n.attributes() {
            if a.namespace() == Some(R) {
                assert!(
                    ids.contains(a.value()),
                    "{} references undeclared {}",
                    a.name(),
                    a.value()
                );
            }
        }
    }
}

#[test]
fn styles_used_by_the_document_exist() {
    let files = write(&sample_book());
    let styles = part_str(&files, "word/styles.xml");
    let sd = Document::parse(&styles).unwrap();
    let defined: HashSet<&str> = sd
        .descendants()
        .filter(|n| n.has_tag_name((W, "style")))
        .filter_map(|n| w_attr(n, "styleId"))
        .collect();
    for needed in [
        "Normal",
        "Heading1",
        "Heading2",
        "Heading3",
        "Heading4",
        "Heading5",
        "Heading6",
        "Hyperlink",
        "TableGrid",
        "ListParagraph",
        "Quote",
        "Code",
        "TOC1",
        "TOC2",
        "TOC3",
        "Title",
    ] {
        assert!(defined.contains(needed), "style {needed} missing");
    }
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    for n in doc.descendants().filter(|n| {
        n.is_element() && matches!(n.tag_name().name(), "pStyle" | "rStyle" | "tblStyle")
    }) {
        let v = w_attr(n, "val").unwrap();
        assert!(defined.contains(v), "document uses undefined style {v}");
    }
    // headings carry outline levels so Word builds a navigation pane
    for l in 0..6 {
        let id = format!("Heading{}", l + 1);
        let style = sd
            .descendants()
            .find(|n| n.has_tag_name((W, "style")) && w_attr(*n, "styleId") == Some(&id))
            .unwrap();
        let lvl = style
            .descendants()
            .find(|n| n.has_tag_name((W, "outlineLvl")))
            .unwrap();
        assert_eq!(w_attr(lvl, "val"), Some(l.to_string().as_str()));
    }
}

#[test]
fn text_headings_links_and_images_of_the_sample_book() {
    let files = write(&sample_book());
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    let text = body_text(&doc);
    assert_sentences(&text, "docx body");
    assert_eq!(
        count(&doc, "hyperlink"),
        3,
        "two internal and one external link"
    );
    let headings: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "pStyle")))
        .filter_map(|n| w_attr(n, "val"))
        .filter(|v| v.starts_with("Heading"))
        .collect();
    assert_eq!(headings, ["Heading1", "Heading1", "Heading1", "Heading2"]);
    // cover first, then the inline picture
    let drawings = count(&doc, "drawing");
    assert_eq!(drawings, 2);
    assert!(files.iter().any(|(n, d)| n.starts_with("word/media/")
        && n.ends_with(".jpeg")
        && d.starts_with(&[0xff, 0xd8])));
    assert!(files.iter().any(|(n, d)| n.starts_with("word/media/")
        && n.ends_with(".png")
        && d.starts_with(&[0x89, b'P'])));
    // the media bytes are exactly the book's images
    let png_bytes = png(40, 30);
    assert!(files.iter().any(|(_, d)| *d == png_bytes));
}

#[test]
fn internal_links_point_at_existing_unique_bookmarks() {
    let files = write(&sample_book());
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    for n in doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "bookmarkStart")))
    {
        assert!(
            names.insert(w_attr(n, "name").unwrap()),
            "duplicate bookmark name"
        );
        assert!(
            ids.insert(w_attr(n, "id").unwrap()),
            "duplicate bookmark id"
        );
        assert!(w_attr(n, "name").unwrap().len() <= 40);
    }
    let ends: HashSet<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "bookmarkEnd")))
        .filter_map(|n| w_attr(n, "id"))
        .collect();
    assert_eq!(ends, ids, "every bookmark is closed");
    let anchors: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "hyperlink")))
        .filter_map(|n| w_attr(n, "anchor"))
        .collect();
    assert_eq!(anchors.len(), 2);
    for a in anchors {
        assert!(names.contains(a), "anchor {a} has no bookmark");
    }
}

#[test]
fn drawing_ids_are_unique_and_extents_positive() {
    let book = book_with(
        &[
            "<p><img src=\"pic.png\" alt=\"one\"/> <img src=\"pic.jpg\"/> <img src=\"anim.gif\"/> <img src=\"pic.png\"/></p>",
        ],
        true,
    );
    let files = write(&book);
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    let mut ids = HashSet::new();
    let mut n = 0;
    for p in doc.descendants().filter(|n| n.tag_name().name() == "docPr") {
        n += 1;
        assert!(ids.insert(p.attribute("id").unwrap()));
    }
    assert_eq!(n, 5, "cover plus four images");
    for e in doc
        .descendants()
        .filter(|n| n.tag_name().name() == "extent")
    {
        let cx: u64 = e.attribute("cx").unwrap().parse().unwrap();
        let cy: u64 = e.attribute("cy").unwrap().parse().unwrap();
        assert!(cx > 0 && cy > 0);
        // never wider than the 6.5 inch text column of the default page
        assert!(cx <= 6 * 914_400 + 457_200, "{cx}");
    }
    // the same picture twice shares one media part and one relationship
    let png_parts = files
        .iter()
        .filter(|(n, _)| n.starts_with("word/media/") && n.ends_with(".png"))
        .count();
    assert_eq!(png_parts, 1);
    assert!(files.iter().any(|(n, _)| n.ends_with(".gif")));
    assert!(part_str(&files, "[Content_Types].xml").contains("image/gif"));
    let alt: Vec<_> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "docPr")
        .filter_map(|n| n.attribute("descr"))
        .collect();
    assert!(alt.contains(&"one"));
}

#[test]
fn run_formatting_is_written_in_schema_order() {
    let book = book_with(
        &[
            "<p><b>b</b><i>i</i><u>u</u><s>s</s><sup>p</sup><sub>q</sub><code>c</code><b><i><u>all</u></i></b></p>",
        ],
        false,
    );
    let files = write(&book);
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    for (tag, n) in [("b", 2), ("i", 2), ("u", 2), ("strike", 1)] {
        let found = doc
            .descendants()
            .filter(|x| x.is_element() && x.has_tag_name((W, "rPr")))
            .filter(|rp| rp.children().any(|c| c.has_tag_name((W, tag))))
            .count();
        assert_eq!(found, n, "runs with <w:{tag}/>");
    }
    let va: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "vertAlign")))
        .filter_map(|n| w_attr(n, "val"))
        .collect();
    assert_eq!(va, ["superscript", "subscript"]);
    assert!(
        xml.contains("Courier New"),
        "code runs use a monospace font"
    );
    // child order inside rPr: b, i, strike, u, vertAlign
    let order = [
        "rStyle",
        "rFonts",
        "b",
        "i",
        "smallCaps",
        "strike",
        "u",
        "vertAlign",
    ];
    for rp in doc.descendants().filter(|n| n.has_tag_name((W, "rPr"))) {
        let idx: Vec<_> = rp
            .children()
            .filter(|c| c.is_element())
            .map(|c| {
                order
                    .iter()
                    .position(|o| *o == c.tag_name().name())
                    .unwrap()
            })
            .collect();
        assert!(
            idx.windows(2).all(|w| w[0] < w[1]),
            "rPr children out of order: {idx:?}"
        );
    }
}

#[test]
fn paragraph_property_order_follows_the_schema() {
    let book = book_with(
        &[
            "<h1 style=\"text-align:center\">H</h1><blockquote><p>q</p></blockquote><ul><li>x</li></ul><hr/><p style=\"text-align:right\">r</p>",
        ],
        false,
    );
    let files = write(&book);
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    let order = [
        "pStyle",
        "keepNext",
        "pageBreakBefore",
        "numPr",
        "pBdr",
        "spacing",
        "ind",
        "jc",
    ];
    for ppr in doc.descendants().filter(|n| n.has_tag_name((W, "pPr"))) {
        let idx: Vec<_> = ppr
            .children()
            .filter(|c| c.is_element())
            .map(|c| {
                order
                    .iter()
                    .position(|o| *o == c.tag_name().name())
                    .unwrap_or_else(|| panic!("unknown {}", c.tag_name().name()))
            })
            .collect();
        assert!(
            idx.windows(2).all(|w| w[0] < w[1]),
            "pPr children out of order: {idx:?}"
        );
    }
    assert!(xml.contains("w:val=\"center\""));
    assert!(xml.contains("w:val=\"right\""));
    assert!(xml.contains("w:pBdr"));
    assert!(xml.contains("<w:pStyle w:val=\"Quote\"/>"));
}

#[test]
fn lists_use_numbering_definitions_and_ordered_lists_restart() {
    let book = book_with(
        &[
            "<ul><li>a<ul><li>nested</li></ul></li><li>b</li></ul><ol><li>one</li><li>two</li></ol><ol><li>again</li></ol>",
        ],
        false,
    );
    let files = write(&book);
    let doc_text = doc_xml(&files);
    let doc = Document::parse(&doc_text).unwrap();
    let used: Vec<(String, String)> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "numPr")))
        .map(|n| {
            let lvl = n.children().find(|c| c.has_tag_name((W, "ilvl"))).unwrap();
            let id = n.children().find(|c| c.has_tag_name((W, "numId"))).unwrap();
            (
                w_attr(lvl, "val").unwrap().to_string(),
                w_attr(id, "val").unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        used,
        [
            ("0", "1"),
            ("1", "1"),
            ("0", "1"),
            ("0", "2"),
            ("0", "2"),
            ("0", "3")
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    let numbering = part_str(&files, "word/numbering.xml");
    let nd = Document::parse(&numbering).unwrap();
    let abstracts: HashSet<_> = nd
        .descendants()
        .filter(|n| n.has_tag_name((W, "abstractNum")))
        .filter_map(|n| w_attr(n, "abstractNumId"))
        .collect();
    assert_eq!(abstracts.len(), 2);
    let nums: HashMap<_, _> = nd
        .descendants()
        .filter(|n| n.has_tag_name((W, "num")))
        .map(|n| {
            let a = n
                .children()
                .find(|c| c.has_tag_name((W, "abstractNumId")))
                .unwrap();
            (w_attr(n, "numId").unwrap(), w_attr(a, "val").unwrap())
        })
        .collect();
    assert_eq!(nums.len(), 3);
    assert_eq!(nums["1"], "0", "bullets");
    assert_eq!(nums["2"], "1", "decimal");
    assert_eq!(nums["3"], "1");
    for abs in nums.values() {
        assert!(abstracts.contains(abs));
    }
    // ordered list definition: decimal at level 0, nine levels
    let dec = nd
        .descendants()
        .find(|n| n.has_tag_name((W, "abstractNum")) && w_attr(*n, "abstractNumId") == Some("1"))
        .unwrap();
    let lvls: Vec<_> = dec
        .children()
        .filter(|c| c.has_tag_name((W, "lvl")))
        .collect();
    assert_eq!(lvls.len(), 9);
    let fmt0 = lvls[0]
        .children()
        .find(|c| c.has_tag_name((W, "numFmt")))
        .unwrap();
    assert_eq!(w_attr(fmt0, "val"), Some("decimal"));
    for id in ["2", "3"] {
        let num = nd
            .descendants()
            .find(|n| n.has_tag_name((W, "num")) && w_attr(*n, "numId") == Some(id))
            .unwrap();
        assert!(
            num.descendants()
                .any(|n| n.has_tag_name((W, "startOverride"))),
            "list {id} restarts at 1"
        );
    }
}

#[test]
fn tables_have_grid_cells_spans_and_header_rows() {
    let book = book_with(
        &[
            "<table><tr><th>H1</th><th>H2</th><th>H3</th></tr><tr><td colspan=\"2\">wide</td><td>x</td></tr><tr><td>only</td></tr></table><p>after</p>",
        ],
        false,
    );
    let files = write(&book);
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    assert_eq!(count(&doc, "tbl"), 1);
    assert_eq!(count(&doc, "tblGrid"), 1);
    let grid: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "gridCol")))
        .collect();
    assert_eq!(grid.len(), 3);
    let rows: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "tr")))
        .collect();
    assert_eq!(rows.len(), 3);
    // every row spans exactly 3 grid columns, every cell holds at least one paragraph
    for r in &rows {
        let mut span = 0;
        for c in r.children().filter(|c| c.has_tag_name((W, "tc"))) {
            assert!(c.children().any(|p| p.has_tag_name((W, "p"))), "empty cell");
            let gs = c
                .descendants()
                .find(|n| n.has_tag_name((W, "gridSpan")))
                .and_then(|n| w_attr(n, "val"))
                .map(|v| v.parse::<usize>().unwrap())
                .unwrap_or(1);
            span += gs;
        }
        assert_eq!(span, 3);
    }
    assert_eq!(count(&doc, "tblHeader"), 1, "first row is a header row");
    // a paragraph follows the table (Word requires one)
    let body = doc
        .descendants()
        .find(|n| n.has_tag_name((W, "body")))
        .unwrap();
    let kids: Vec<_> = body
        .children()
        .filter(|c| c.is_element())
        .map(|c| c.tag_name().name().to_string())
        .collect();
    assert_eq!(
        kids,
        ["p", "tbl", "p", "p", "sectPr"],
        "bookmark paragraph, table, the mandatory paragraph after it, text"
    );
    let text = body_text(&doc);
    assert!(text.contains("H1") && text.contains("wide") && text.contains("after"));
}

#[test]
fn chapters_start_on_new_pages() {
    let book = book_with(&["<p>one</p>", "<p>two</p>", "<p>three</p>"], false);
    let doc_text = doc_xml(&write(&book));
    let doc = Document::parse(&doc_text).unwrap();
    assert_eq!(count(&doc, "pageBreakBefore"), 2);
    let first_p = doc
        .descendants()
        .find(|n| n.has_tag_name((W, "p")))
        .unwrap();
    assert!(
        !first_p
            .descendants()
            .any(|n| n.has_tag_name((W, "pageBreakBefore")))
    );

    let with_cover = book_with(&["<p>one</p>", "<p>two</p>"], true);
    let doc_text = doc_xml(&write(&with_cover));
    let doc = Document::parse(&doc_text).unwrap();
    assert_eq!(
        count(&doc, "pageBreakBefore"),
        2,
        "after the cover and between chapters"
    );
}

#[test]
fn page_size_margins_and_font_options_are_applied() {
    let book = book_with(&["<p>x</p>"], false);
    let files = write_with(
        &book,
        DocxOptions {
            page: PageSize::A4,
            margin_twips: 1000,
            font: "Georgia".into(),
            font_size_pt: 10.5,
            toc: false,
        },
    );
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    let sect = doc
        .descendants()
        .find(|n| n.has_tag_name((W, "sectPr")))
        .unwrap();
    let pg = sect
        .children()
        .find(|n| n.has_tag_name((W, "pgSz")))
        .unwrap();
    assert_eq!(
        (w_attr(pg, "w"), w_attr(pg, "h")),
        (Some("11906"), Some("16838"))
    );
    let mar = sect
        .children()
        .find(|n| n.has_tag_name((W, "pgMar")))
        .unwrap();
    for side in ["top", "right", "bottom", "left"] {
        assert_eq!(w_attr(mar, side), Some("1000"));
    }
    let styles = part_str(&files, "word/styles.xml");
    let sd = Document::parse(&styles).unwrap();
    let fonts = sd
        .descendants()
        .find(|n| n.has_tag_name((W, "rFonts")))
        .unwrap();
    assert_eq!(w_attr(fonts, "ascii"), Some("Georgia"));
    let sz = sd
        .descendants()
        .find(|n| n.has_tag_name((W, "sz")))
        .unwrap();
    assert_eq!(w_attr(sz, "val"), Some("21"));

    let custom = write_with(
        &book,
        DocxOptions {
            page: PageSize::Custom(8000, 10000),
            ..Default::default()
        },
    );
    let xml = doc_xml(&custom);
    assert!(xml.contains("w:w=\"8000\" w:h=\"10000\""));
}

#[test]
fn table_of_contents_field_is_optional() {
    let book = nested_toc_book();
    let off = doc_xml(&write(&book));
    assert!(!off.contains("instrText"));
    let files = write_with(
        &book,
        DocxOptions {
            toc: true,
            ..Default::default()
        },
    );
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    let instr: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "instrText")))
        .collect();
    assert_eq!(instr.len(), 1);
    assert!(instr[0].text().unwrap().contains("TOC"));
    let kinds: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "fldChar")))
        .filter_map(|n| w_attr(n, "fldCharType"))
        .collect();
    assert_eq!(kinds, ["begin", "separate", "end"]);
    let styles: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "pStyle")))
        .filter_map(|n| w_attr(n, "val"))
        .filter(|v| v.starts_with("TOC"))
        .collect();
    assert_eq!(styles, ["TOCHeading", "TOC1", "TOC2", "TOC1", "TOC2"]);
    // every TOC entry is a link to a bookmark that exists
    let names: HashSet<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "bookmarkStart")))
        .filter_map(|n| w_attr(n, "name"))
        .collect();
    let toc_links: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "hyperlink")))
        .filter_map(|n| w_attr(n, "anchor"))
        .collect();
    assert!(toc_links.len() >= 4);
    for l in toc_links {
        assert!(names.contains(l));
    }
    let text = body_text(&doc);
    for t in ["Contents", "Part I", "Part II", "A section"] {
        assert!(text.contains(t), "{t}");
    }
}

#[test]
fn awkward_text_cannot_break_the_xml() {
    let book = book_with(
        &[
            "<p>&lt;tag&gt; &amp; \"quotes\" 'single' \u{1}\u{8}ctl\u{0b} \u{feff}bom \u{fffe} 😀 עברית</p><pre>tab\there\nline two</pre>",
        ],
        false,
    );
    let files = write(&book);
    for (n, d) in &files {
        if n.ends_with(".xml") || n.ends_with(".rels") {
            Document::parse(std::str::from_utf8(d).unwrap()).unwrap_or_else(|e| panic!("{n}: {e}"));
        }
    }
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    let text = body_text(&doc);
    assert!(text.contains("<tag> & \"quotes\" 'single'"));
    assert!(text.contains("ctl"));
    assert!(!text.contains('\u{1}'));
    assert!(text.contains("😀 עברית"));
    assert!(text.contains("tab\there"));
    assert_eq!(
        count(&doc, "br"),
        1,
        "pre lines are joined with line breaks"
    );
}

#[test]
fn metadata_goes_to_the_doc_properties() {
    let files = write(&sample_book());
    let core = part_str(&files, "docProps/core.xml");
    let doc = Document::parse(&core).unwrap();
    let get = |tag: &str| {
        doc.descendants()
            .find(|n| n.tag_name().name() == tag)
            .and_then(|n| n.text())
            .map(str::to_string)
    };
    assert_eq!(get("title").as_deref(), Some("The Test Book: A Story"));
    assert_eq!(
        get("creator").as_deref(),
        Some("Ada Lovelace; Grace Hopper")
    );
    assert_eq!(get("keywords").as_deref(), Some("testing, fiction"));
    assert_eq!(get("language").as_deref(), Some("en"));
    assert!(
        get("description")
            .unwrap()
            .contains("short description & more")
    );
    let created = get("created").unwrap();
    assert!(
        chrono::DateTime::parse_from_rfc3339(&created).is_ok(),
        "{created}"
    );
    let app = part_str(&files, "docProps/app.xml");
    assert!(app.contains("<Application>Bookx</Application>"));
    assert!(app.contains("<Company>Bookx Press</Company>"));
}

#[test]
fn an_empty_book_still_produces_a_valid_document() {
    let book = book_with(&["<div></div>"], false);
    let files = write(&book);
    let xml = doc_xml(&files);
    let doc = Document::parse(&xml).unwrap();
    assert_eq!(count(&doc, "p"), 1);
    assert_eq!(count(&doc, "sectPr"), 1);
}

#[test]
fn missing_images_do_not_leave_dangling_relationships() {
    let book = book_with(
        &["<p>t <img src=\"nope.png\" alt=\"gone\"/> <img src=\"pic.png\"/></p>"],
        false,
    );
    let files = write(&book);
    let rels = part_str(&files, "word/_rels/document.xml.rels");
    assert_eq!(rels.matches("/image\"").count(), 1);
    assert!(body_text(&Document::parse(&doc_xml(&files)).unwrap()).contains("gone"));
}

#[test]
fn quotes_and_code_use_their_styles() {
    let book = book_with(
        &["<blockquote><blockquote><p>deep</p></blockquote></blockquote><pre>code</pre>"],
        false,
    );
    let xml = doc_xml(&write(&book));
    assert!(xml.contains("<w:pStyle w:val=\"Quote\"/>"));
    assert!(
        xml.contains("<w:ind w:left=\"1440\"/>"),
        "nested quotes indent twice"
    );
    assert!(xml.contains("<w:pStyle w:val=\"Code\"/>"));
}

// ---- oracles ---------------------------------------------------------------------------------

#[test]
fn round_trips_through_the_docx_reader() {
    use bookx_core::contracts::FormatReader;
    let bytes = DocxWriter::default().write(&sample_book()).unwrap();
    let archive = bookx_formats::docx::DocxReader.read(&bytes).unwrap();
    let text = epub_text(archive);
    assert_sentences(&text, "docx round trip");
}

#[test]
fn round_trip_keeps_headings_and_the_toc() {
    use bookx_core::contracts::FormatReader;
    let bytes = DocxWriter::default().write(&nested_toc_book()).unwrap();
    let archive = bookx_formats::docx::DocxReader.read(&bytes).unwrap();
    let book = bookx_core::book::OpenBook::from_archive(archive).unwrap();
    let titles: Vec<String> = bookx_core::toc::flatten(&book.toc())
        .into_iter()
        .map(|f| f.title)
        .collect();
    for t in ["Chapter One", "Chapter Two", "Chapter Three"] {
        assert!(titles.iter().any(|x| x == t), "toc lacks {t}: {titles:?}");
    }
}

#[test]
fn calibre_reads_our_docx() {
    let bytes = DocxWriter::default().write(&sample_book()).unwrap();
    let Some(epub) = calibre(&bytes, "ours.docx", "back.epub", &[]) else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    let archive = bookx_core::archive::Archive::from_bytes(epub).unwrap();
    let text = epub_text(archive.clone());
    assert_sentences(&text, "Calibre read of our docx");
    // structure: headings, emphasis, links and both images survive Calibre's reader
    let all: String = archive
        .names()
        .iter()
        .filter(|n| n.ends_with(".html"))
        .filter_map(|n| archive.get_string(n))
        .collect();
    assert!(all.contains("<h1"), "headings");
    assert!(all.contains("<h2"));
    assert!(
        all.contains("<i class") && all.contains("<b class"),
        "emphasis"
    );
    assert!(all.contains("https://example.org/x"));
    assert!(all.matches("<img").count() >= 2, "cover and picture");
    assert!(
        all.contains("href=\"index_split_00"),
        "internal links survive"
    );
}

#[test]
fn calibre_reads_lists_and_tables_of_our_docx() {
    let book = book_with(
        &[
            "<h1>H</h1><ul><li>apple</li><li>pear</li></ul><ol><li>first</li><li>second</li></ol><table><tr><th>K</th><th>V</th></tr><tr><td>a</td><td>b</td></tr></table>",
        ],
        false,
    );
    let bytes = DocxWriter::default().write(&book).unwrap();
    let Some(epub) = calibre(&bytes, "ours.docx", "back.epub", &[]) else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    let archive = bookx_core::archive::Archive::from_bytes(epub).unwrap();
    let html: String = archive
        .names()
        .iter()
        .filter(|n| n.ends_with(".html"))
        .filter_map(|n| archive.get_string(n))
        .collect();
    assert!(
        html.contains("<ul") && html.contains("<ol") || html.contains("apple"),
        "{html}"
    );
    assert!(html.contains("<table"), "table survives: {html}");
    let text = epub_text(archive);
    for w in ["apple", "pear", "first", "second", "K", "V"] {
        assert!(text.contains(w), "{w} in {text}");
    }
}

#[test]
fn structure_matches_calibres_own_docx_package() {
    // Calibre's DOCX output of the same HTML has the same set of mandatory parts.
    let html = "<html><head><title>T</title></head><body><h1>H</h1><p>x</p></body></html>";
    let Some(theirs) = calibre(html.as_bytes(), "a.html", "a.docx", &[]) else {
        eprintln!("skipped: Calibre not available");
        return;
    };
    let their_names: HashSet<String> = unzip(&theirs).into_iter().map(|f| f.0).collect();
    let ours = write(&book_with(&["<h1>H</h1><p>x</p>"], false));
    let our_names: HashSet<String> = ours.into_iter().map(|f| f.0).collect();
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "docProps/core.xml",
        "docProps/app.xml",
        "word/document.xml",
        "word/styles.xml",
        "word/numbering.xml",
        "word/_rels/document.xml.rels",
    ] {
        assert!(their_names.contains(required), "Calibre lacks {required}?");
        assert!(our_names.contains(required), "we lack {required}");
    }
}
