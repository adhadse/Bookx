#![forbid(unsafe_code)]
//! GUI-independent engine: EPUB parsing/editing, conversion, library catalogue.

pub mod annotations;
pub mod archive;
pub mod book;
pub mod check;
pub mod contracts;
pub mod convert;
pub mod doc;
pub mod drm;
pub mod files;
pub mod format;
pub mod formats;
pub mod fsops;
pub mod history;
pub mod html;
pub mod inbox;
pub mod layout;
pub mod library;
pub mod merge;
pub mod opf;
pub mod quality;
pub mod query;
pub mod toc;
pub mod util;

#[cfg(test)]
mod real_world {
    use super::book::OpenBook;
    use super::check::check_book;
    use super::toc;
    use std::path::PathBuf;

    fn samples() -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Ok(dir) = std::env::var("BOOKX_TEST_EPUBS") {
            super::library::collect_book_files(&PathBuf::from(dir), true, &mut v);
        }
        v
    }

    /// Run with `BOOKX_TEST_EPUBS=~/CalibreLibrary cargo test real_world -- --nocapture`
    #[test]
    fn edit_roundtrip_on_real_books() {
        for path in samples()
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "epub"))
            .take(8)
        {
            let mut book =
                OpenBook::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let md0 = book.metadata().clone();
            let toc0 = book.toc();
            println!(
                "{} | {} | {:?} | spine {} | toc {}",
                path.display(),
                md0.title,
                md0.authors,
                book.spine_paths().len(),
                toc0.len()
            );

            let mut md = md0.clone();
            md.title = format!("{} (edited)", md.title);
            md.tags.push("bookx-test".into());
            md.series = "Test Series".into();
            md.series_index = 2.5;
            md.rating = 8;
            md.publisher = "Pub & <Co>".into();
            md.description = "A \"description\" with <b>markup</b> & more".into();
            md.set_identifiers_from_string("isbn:9781234567890, doi:10.1/x");
            book.set_metadata(md.clone()).unwrap();

            let bytes = book.archive.to_bytes().unwrap();
            let again = OpenBook::from_archive(super::archive::Archive::from_bytes(bytes).unwrap())
                .unwrap();
            let m = again.metadata();
            assert_eq!(m.title, md.title);
            assert_eq!(m.tags, md.tags);
            assert_eq!(m.series, "Test Series");
            assert_eq!(m.series_index, 2.5);
            assert_eq!(m.rating, 8);
            assert_eq!(m.publisher, md.publisher);
            assert_eq!(m.description, md.description);
            assert_eq!(m.authors, md0.authors);
            assert!(m.identifier("isbn") == Some("9781234567890"));
            assert!(m.identifier("doi") == Some("10.1/x"));
            assert_eq!(again.spine_paths(), book.spine_paths());

            // TOC rewrite roundtrip
            let t = toc::flatten(&toc0);
            if !t.is_empty() {
                let mut b2 = OpenBook::open(&path).unwrap();
                let pkg = b2.package.clone();
                toc::write_toc(&mut b2.archive, &pkg, &toc0).unwrap();
                let back = toc::flatten(&b2.toc());
                assert_eq!(back.len(), t.len(), "toc length for {}", path.display());
                for (a, b) in back.iter().zip(t.iter()) {
                    assert_eq!(a.href, b.href);
                    assert_eq!(a.title, b.title);
                }
            }

            // cover replacement
            let mut b3 = OpenBook::open(&path).unwrap();
            let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
            b3.set_cover(png.to_vec(), "png").unwrap();
            assert_eq!(b3.cover_bytes(), Some(&png[..]));
            let out = path.with_file_name("/tmp/opencode/bookx-test.epub");
            let _ = std::fs::create_dir_all("/tmp/opencode");
            b3.archive.save(&out).unwrap();

            let issues = check_book(&book.archive);
            for i in issues.iter().take(4) {
                println!("   {:?} {} {}", i.severity, i.file, i.message);
            }
        }
    }
}

#[cfg(test)]
mod library_flow {
    use super::library::{AddOutcome, Library, SortKey};
    use std::path::PathBuf;

    /// Creates an EPUB from Markdown, then exercises add / edit / search / remove.
    #[test]
    fn add_edit_search_remove() {
        let tmp = std::env::temp_dir().join(format!("bookx-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let md = tmp.join("My Story.md");
        std::fs::write(&md, "---\ntitle: \"The Story\"\nauthor: \"Ada Lovelace\"\n---\n# One\n\nHello world.\n\n# Two\n\nBye *now*.\n").unwrap();
        let txt = tmp.join("plain.txt");
        std::fs::write(
            &txt,
            "Chapter 1\n\nSome text here.\n\nChapter 2\n\nMore text.\n",
        )
        .unwrap();

        let backend = super::library::MemoryBackend::new();
        let mut lib = Library::with_backend(&tmp.join("lib"), Box::new(backend.clone())).unwrap();
        let AddOutcome::Added(id) = lib.add_path(&md, false).unwrap() else {
            panic!("not added")
        };
        assert!(matches!(
            lib.add_path(&md, false).unwrap(),
            AddOutcome::Duplicate(_)
        ));
        let AddOutcome::Added(id2) = lib.add_path(&txt, false).unwrap() else {
            panic!("not added")
        };
        assert_eq!(lib.books.len(), 2);
        assert_eq!(lib.get(id).unwrap().meta.title, "The Story");
        assert_eq!(lib.get(id).unwrap().meta.authors, vec!["Ada Lovelace"]);

        // edit metadata: moves the files
        let mut m = lib.get(id).unwrap().meta.clone();
        m.title = "Renamed Story".into();
        m.tags = vec!["fiction".into(), "test".into()];
        m.series = "Saga".into();
        m.authors = vec!["Grace Hopper".into()];
        lib.update_metadata(id, m).unwrap();
        let b = lib.get(id).unwrap();
        assert!(
            b.file.starts_with("Grace Hopper/Renamed Story"),
            "file was {}",
            b.file
        );
        assert!(lib.book_path(b).exists());
        assert_eq!(lib.open_book(id).unwrap().metadata().title, "Renamed Story");

        // search
        assert_eq!(lib.search("tag:fiction", SortKey::Title, true), vec![id]);
        assert_eq!(
            lib.search("series:=Saga author:hopper", SortKey::Title, true),
            vec![id]
        );
        assert_eq!(lib.search("-tag:fiction", SortKey::Title, true), vec![id2]);
        assert_eq!(
            lib.search("renamed OR plain", SortKey::Title, true).len(),
            2
        );
        assert_eq!(lib.search("status:unread", SortKey::Title, true).len(), 2);

        // persistence
        lib.save().unwrap();
        let reopened = Library::with_backend(&tmp.join("lib"), Box::new(backend.clone())).unwrap();
        assert_eq!(reopened.books.len(), 2);

        // convert
        let book = lib.open_book(id).unwrap();
        let md_out = super::convert::export_book(&book, super::convert::ExportFormat::Markdown);
        assert!(md_out.contains("Hello world."));

        lib.remove(&[id, id2], true).unwrap();
        assert!(lib.books.is_empty());
        assert!(!PathBuf::from(&tmp).join("lib/Grace Hopper").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
