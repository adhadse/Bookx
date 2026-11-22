//! Filename guessing, templates, identifiers and duplicate detection working together.

use bookx_core::library::Book;
use bookx_core::opf::Metadata;
use bookx_sources::dedup::{DEFAULT_THRESHOLD, find_duplicates, merge_metadata};
use bookx_sources::filename::{default_patterns, guess};
use bookx_sources::identifiers::{IdKind, ids_of, parse};
use bookx_sources::isbn::extract_isbns;
use bookx_sources::template::render_save_path;
use chrono::Utc;

fn book(id: u64, meta: Metadata) -> Book {
    Book {
        id,
        file: format!("{id}.epub"),
        cover: None,
        meta,
        added: Utc::now(),
        modified: Utc::now(),
        last_read: None,
        size: 0,
        progress: 0.0,
        position: None,
        bookmarks: vec![],
        highlights: vec![],
        formats: vec![],
        resources: vec![],
        drm: String::new(),
        custom: Default::default(),
        marked: false,
    }
}

#[test]
fn messy_file_names_become_clean_library_paths() {
    let names = [
        "Terry Pratchett - Discworld #30 - The Wee Free Men (z-lib.org).epub",
        "Pratchett, Terry - Wee Free Men [retail].epub",
        "The_Wee_Free_Men_(Terry_Pratchett).mobi",
    ];
    for n in names {
        let md = guess(n, &default_patterns()).into_metadata();
        assert!(md.title.contains("Wee Free Men"), "{n}: {md:?}");
        assert_eq!(md.authors, vec!["Terry Pratchett"], "{n}");
        let path = render_save_path("{author_sort:first_author}/{title}", &md, "epub").unwrap();
        assert_eq!(path[0], "Pratchett, Terry");
        assert!(path[1].ends_with(".epub") && !path[1].contains("z-lib"));
    }
}

#[test]
fn guessed_metadata_feeds_duplicate_detection() {
    let files = [
        "J.R.R. Tolkien - The Hobbit.epub",
        "Tolkien, John Ronald Reuel - Hobbit, The (1937).mobi",
        "Frank Herbert - Dune.epub",
        "Herbert, Frank - Dune [OCR].pdf",
        "Isaac Asimov - Foundation.epub",
    ];
    let books: Vec<Book> = files
        .iter()
        .enumerate()
        .map(|(i, f)| book(i as u64 + 1, guess(f, &default_patterns()).into_metadata()))
        .collect();
    let sets = find_duplicates(&books, DEFAULT_THRESHOLD);
    let ids: Vec<Vec<u64>> = sets.into_iter().map(|s| s.ids).collect();
    assert_eq!(ids, vec![vec![1, 2], vec![3, 4]]);
}

#[test]
fn merging_duplicates_keeps_the_best_of_each() {
    let mut a = guess("J.R.R. Tolkien - The Hobbit.epub", &default_patterns()).into_metadata();
    a.tags = vec!["Fantasy".into()];
    let mut b = guess(
        "Tolkien, John Ronald Reuel - The Hobbit.mobi",
        &default_patterns(),
    )
    .into_metadata();
    b.publisher = "Allen & Unwin".into();
    b.tags = vec!["fantasy".into(), "Classic".into()];
    b.identifiers = ids_of(&[bookx_core::opf::Identifier {
        scheme: String::new(),
        value: "978-0-618-26030-0".into(),
        xml_id: None,
    }])
    .iter()
    .map(|i| i.to_identifier())
    .collect();
    let m = merge_metadata(&a, &[b]);
    assert_eq!(m.authors, vec!["J.R.R. Tolkien"]);
    assert_eq!(m.publisher, "Allen & Unwin");
    assert_eq!(m.tags, vec!["Fantasy", "Classic"]);
    assert_eq!(m.identifier("isbn"), Some("9780618260300"));
}

#[test]
fn identifiers_found_in_text_and_urls_agree() {
    let blurb = "Copyright 2003. ISBN 978-0-306-40615-7. Also see https://www.goodreads.com/book/show/34497.The_Wee_Free_Men";
    let isbns = extract_isbns(blurb);
    assert_eq!(isbns.len(), 1);
    let id = parse(&isbns[0]).unwrap();
    assert_eq!(id.kind, IdKind::Isbn);
    let gr = parse("https://www.goodreads.com/book/show/34497.The_Wee_Free_Men").unwrap();
    assert_eq!(gr.value, "34497");
    assert_eq!(parse(&gr.url()).unwrap(), gr);
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn hostile_metadata_never_escapes_the_library_directory() {
    let nasty = [
        "..",
        "../..",
        "/etc/passwd",
        "a/../../b",
        "C:\\Windows",
        "con",
        "nul.txt",
        ".hidden",
        "x\u{0}y",
        "😀/😀",
        "\u{202E}gnp.exe",
        "   ",
        "",
    ];
    let mut r = Rng(0xDEADBEEF);
    for _ in 0..500 {
        let pick = |r: &mut Rng| nasty[(r.next() % nasty.len() as u64) as usize].to_string();
        let md = Metadata {
            title: pick(&mut r),
            authors: vec![pick(&mut r), pick(&mut r)],
            series: pick(&mut r),
            series_index: (r.next() % 5) as f64,
            publisher: pick(&mut r),
            ..Default::default()
        };
        let comps = render_save_path(
            "{author_sort}/{series?:}/{series_index:pad(3)} {title}",
            &md,
            "epub",
        )
        .unwrap();
        assert!(!comps.is_empty());
        for c in &comps {
            assert!(
                !c.is_empty() && c != "." && c != "..",
                "{md:?} -> {comps:?}"
            );
            assert!(!c.contains(['/', '\\', ':', '\0']), "{md:?} -> {comps:?}");
            assert!(!c.starts_with('.'), "{md:?} -> {comps:?}");
            assert!(c.len() <= 240 && c.chars().count() <= 120);
        }
    }
}
