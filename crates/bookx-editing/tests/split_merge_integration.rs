//! Split and merge on real EPUBs, validated from the zip bytes.

mod common;

use bookx_editing::markup::{defined_ids, normalized_text};
use bookx_editing::split_merge::{SplitSpec, merge_files, split_file};

fn big_chapter() -> String {
    let mut s = String::new();
    for i in 1..=6 {
        s.push_str(&format!("<h1 id=\"part{i}\">Part {i}</h1><p>Words of part {i}.</p><div class=\"box\" id=\"box{i}\"><p id=\"in{i}\">Inside {i} <a href=\"#part{}\">next</a></p></div>", i % 6 + 1));
    }
    s
}

#[test]
fn split_by_headings_ids_and_page_breaks_then_merge_back() {
    for spec in [
        SplitSpec::Headings(1),
        SplitSpec::AtIds(vec!["in2".into(), "part5".into()]),
        SplitSpec::PageBreaks,
    ] {
        let mut chapter = big_chapter();
        if spec == SplitSpec::PageBreaks {
            chapter = chapter.replace(
                "<h1 id=\"part3\">",
                "<h1 id=\"part3\" style=\"page-break-before: always\">",
            );
            chapter = chapter.replace(
                "<h1 id=\"part5\">",
                "<h1 id=\"part5\" style=\"break-before: page\">",
            );
        }
        let mut book = common::book_from_bodies(&[
            "<p>Intro <a href=\"chapter-2.xhtml#in4\">deep link</a> <a href=\"chapter-2.xhtml\">whole</a></p>".to_string(),
            chapter,
            "<p>Outro <a href=\"chapter-2.xhtml#part6\">to part 6</a></p>".to_string(),
        ]);
        let text = common::spine_text(&book);
        let joined = text.join(" ");
        let toc = vec![
            bookx_core::toc::TocEntry {
                title: "Part 4".into(),
                href: "OEBPS/chapter-2.xhtml#part4".into(),
                children: vec![],
            },
            bookx_core::toc::TocEntry {
                title: "Intro".into(),
                href: "OEBPS/chapter-1.xhtml".into(),
                children: vec![],
            },
        ];
        bookx_core::toc::write_toc(&mut book.archive, &book.package, &toc).unwrap();
        common::validate_epub(&book.archive.to_bytes().unwrap());

        let res = split_file(&mut book, "OEBPS/chapter-2.xhtml", &spec).unwrap();
        assert!(res.parts.len() >= 3, "{spec:?}: {:?}", res.parts);
        let files = common::validate_epub(&book.archive.to_bytes().unwrap());
        assert_eq!(common::spine_text(&book).join(" "), joined, "{spec:?}");
        // every id exists exactly once across the parts and the TOC follows its target
        let mut ids: Vec<String> = Vec::new();
        for p in &res.parts {
            ids.extend(defined_ids(std::str::from_utf8(&files[p]).unwrap()));
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "{spec:?}: ids must stay unique");
        for i in 1..=6 {
            assert!(
                ids.contains(&format!("part{i}")) && ids.contains(&format!("in{i}")),
                "{spec:?}: part{i}"
            );
        }
        let reopened = common::reopen(&book);
        let t = reopened.toc();
        let part4 = t.iter().find(|e| e.title == "Part 4").unwrap();
        let holder = res
            .parts
            .iter()
            .find(|p| {
                defined_ids(std::str::from_utf8(&files[*p]).unwrap()).contains(&"part4".to_string())
            })
            .unwrap();
        assert_eq!(part4.href, format!("{holder}#part4"));

        // back together
        merge_files(&mut book, &res.parts).unwrap();
        common::validate_epub(&book.archive.to_bytes().unwrap());
        assert_eq!(common::spine_text(&book), text, "{spec:?} after merge");
        assert_eq!(book.spine_paths().len(), 3);
        let merged = book.archive.get_string("OEBPS/chapter-2.xhtml").unwrap();
        assert_eq!(normalized_text(&merged), text[1]);
        assert!(
            book.toc()
                .iter()
                .any(|e| e.href == "OEBPS/chapter-2.xhtml#part4")
        );
    }
}

#[test]
fn merge_chapters_across_the_book() {
    let mut book = common::book_from_bodies(&[
        "<h1 id=\"t\">One</h1><p id=\"x\">a</p>".to_string(),
        "<h1 id=\"t\">Two</h1><p id=\"x\">b <a href=\"#t\">self</a></p>".to_string(),
        "<h1 id=\"t\">Three</h1><p>c <a href=\"chapter-2.xhtml#t\">to two</a> <a href=\"chapter-1.xhtml#x\">to one x</a></p>".to_string(),
    ]);
    let before = common::spine_text(&book).join(" ");
    let r = merge_files(
        &mut book,
        &[
            "OEBPS/chapter-1.xhtml".into(),
            "OEBPS/chapter-2.xhtml".into(),
            "OEBPS/chapter-3.xhtml".into(),
        ],
    )
    .unwrap();
    assert_eq!(
        r.renamed_ids.len(),
        3,
        "t and x renamed in chapter 2, t in chapter 3: {:?}",
        r.renamed_ids
    );
    let files = common::validate_epub(&book.archive.to_bytes().unwrap());
    assert_eq!(book.spine_paths(), vec!["OEBPS/chapter-1.xhtml"]);
    assert_eq!(common::spine_text(&book).join(" "), before);
    let merged = std::str::from_utf8(&files["OEBPS/chapter-1.xhtml"]).unwrap();
    assert!(merged.contains("href=\"#t-2\">self"), "{merged}");
    assert!(merged.contains("chapter-1.xhtml#t-2\">to two"), "{merged}");
    assert!(
        merged.contains("chapter-1.xhtml#x\">to one x") || merged.contains("href=\"#x\">to one x"),
        "{merged}"
    );
}
