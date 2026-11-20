//! Glue for keeping highlights where their text is: the one-time pass when a library is opened and
//! "find in the book again" for a note that lost its place. The work is `bookx_core::anchor`; no
//! GTK in here, so it is unit tested.

use std::path::{Path, PathBuf};

use crate::core::anchor::{self, BookChapters, Repair, RepairReport};
use crate::core::library::Library;
use crate::i18n::{ntr, trf};

/// Bumped when how highlights are numbered or checked changes, so every library is checked once more.
const PASS: &str = "highlight-anchors-v1";

fn marker(root: &Path) -> PathBuf {
    root.join(crate::core::history::STATE_DIR).join(PASS)
}

/// Checks every stored highlight against its book once per library (the marker file says it was
/// done). Needs a writable library. One undo step in the operation log. `None` when it was done
/// before, nothing needed checking or the library is read-only.
pub fn one_time_pass(lib: &mut Library) -> Option<RepairReport> {
    if lib.is_read_only() || marker(&lib.root).exists() {
        return None;
    }
    let report = lib.repair_highlights(None).ok()?;
    let m = marker(&lib.root);
    if let Some(d) = m.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(
        m,
        format!(
            "checked {} moved {} unplaced {}\n",
            report.checked, report.moved, report.unanchored
        ),
    );
    (report.changed() > 0).then_some(report)
}

/// What to tell the user after a pass that changed something.
pub fn message(r: &RepairReport) -> String {
    let moved = r.moved + r.anchored;
    if r.unanchored == 0 {
        ntr(
            "Fixed the place of {} highlight (it was off from its text)",
            "Fixed the place of {} highlights (they were off from their text)",
            moved as u64,
        )
    } else {
        trf(
            "Re-checked your highlights: {} moved to where their text is, {} could not be found in the book and are kept as notes without a place",
            &[&moved, &r.unanchored],
        )
    }
}

/// Looks for the text of one highlight in its book again and puts it back on the page. False when
/// the text is still nowhere (nothing changes then).
pub fn relocate_one(lib: &mut Library, book_id: u64, highlight_id: &str) -> bool {
    let Ok(open) = lib.open_book(book_id) else {
        return false;
    };
    let chapters = BookChapters::new(open);
    let Some(h) = lib
        .get_mut(book_id)
        .and_then(|b| b.highlights.iter_mut().find(|h| h.id == highlight_id))
    else {
        return false;
    };
    let mut trial = h.clone();
    if !matches!(
        anchor::repair_one(&chapters, &mut trial),
        Repair::Anchored | Repair::Moved
    ) {
        return false;
    }
    *h = trial;
    lib.save().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::convert::{create_epub, Chapter};
    use crate::core::library::{AddOutcome, Highlight};
    use crate::core::opf::Metadata;

    fn lib_with_book(dir: &Path, paragraphs: usize) -> (Library, u64, String) {
        let body: String = (0..paragraphs)
            .map(|i| format!("<p>Paragraph number {i} has some words in it.</p>"))
            .collect::<String>()
            + "<p><em>Independent deployability</em> is the idea that we can make a change to a microservice, deploy it, and release that change to our users.</p>";
        let md = Metadata {
            title: "Micro".into(),
            authors: vec!["Sam".into()],
            language: "en".into(),
            ..Default::default()
        };
        let ch = [Chapter {
            title: "One".into(),
            body: format!("<h1>One</h1>{body}"),
        }];
        let src = dir.join("m.epub");
        std::fs::write(&src, create_epub(&md, &ch, None).to_bytes().unwrap()).unwrap();
        let mut lib = Library::in_memory(&dir.join("lib")).unwrap();
        let AddOutcome::Added(id) = lib.add_path(&src, true).unwrap() else {
            panic!("not added")
        };
        let quote = "Independent deployability is the idea that we can make a change to a microservice, deploy it, and release that change to our users.".to_string();
        (lib, id, quote)
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-hlrepair-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn hl(id: &str, start: i32, end: i32, text: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start,
            end,
            color: "yellow".into(),
            style: String::new(),
            note: "kept".into(),
            text: text.into(),
            created: chrono::Utc::now(),
        }
    }

    /// The owner's bug: offsets counted in `Doc::to_text` (blank lines between paragraphs,
    /// upper-cased headings) were stored for the reader, which counts one line break per
    /// paragraph, so the highlight was painted as many characters late as there are blocks before it.
    #[test]
    fn imported_highlights_are_stored_in_the_readers_numbers() {
        let d = tmp();
        let (lib, id, quote) = lib_with_book(&d, 33);
        let locator = crate::device_sync::book_locator(&lib);
        use crate::device_sync::Locator;
        let (spine, a, b) = locator.locate(id, &quote).unwrap();
        let open = lib.open_book(id).unwrap();
        let doc = open.chapter_doc(&open.spine_paths()[spine]).unwrap();
        let text: Vec<char> = doc.reader_text().chars().collect();
        let got: String = text[a as usize..b as usize].iter().collect();
        assert_eq!(
            got, quote,
            "the stored range holds the quote in the reader's text"
        );
        // the old numbers (to_text) were ahead by one per block before the quote
        let old_start = doc
            .to_text()
            .find("Independent deployability")
            .map(|byte| doc.to_text()[..byte].chars().count())
            .unwrap();
        assert_eq!(
            old_start as i32 - a,
            34,
            "33 paragraphs and the heading came before it"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_pass_fixes_shifted_highlights_keeps_the_rest_and_can_be_undone() {
        let d = tmp();
        let (mut lib, id, quote) = lib_with_book(&d, 33);
        let open = lib.open_book(id).unwrap();
        let doc = open.chapter_doc(&open.spine_paths()[0]).unwrap();
        let rt = doc.reader_text();
        let real = rt.find(&quote).map(|b| rt[..b].chars().count()).unwrap() as i32;
        let len = quote.chars().count() as i32;
        let before = vec![
            hl("shifted", real + 34, real + 34 + len, &quote),
            hl("fine", real, real + len, &quote),
            hl("gone", 3, 20, "A sentence that is not in this book at all"),
        ];
        lib.get_mut(id).unwrap().highlights = before.clone();
        let r = one_time_pass(&mut lib).expect("changed something");
        assert_eq!((r.moved, r.unanchored, r.fine), (1, 1, 1));
        let hs = &lib.get(id).unwrap().highlights;
        assert_eq!((hs[0].start, hs[0].end), (real, real + len));
        assert_eq!((hs[1].start, hs[1].end), (real, real + len));
        assert!(anchor::is_unanchored(&hs[2]));
        assert_eq!(
            anchor::hint_of(&hs[2]),
            (0, 3, 20),
            "old offsets stay in the record"
        );
        assert_eq!(hs[2].note, "kept");
        assert!(hs.iter().all(|h| h.note == "kept"));
        // once per library
        assert!(one_time_pass(&mut lib).is_none());
        // undo restores all of them as they were
        lib.undo_last().unwrap();
        assert_eq!(lib.get(id).unwrap().highlights, before);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_note_without_a_place_can_be_found_again() {
        let d = tmp();
        let (mut lib, id, quote) = lib_with_book(&d, 3);
        let mut h = hl("n", -1, -1, &quote);
        h.spine = 0;
        lib.get_mut(id).unwrap().highlights = vec![h];
        assert!(relocate_one(&mut lib, id, "n"));
        let h = &lib.get(id).unwrap().highlights[0];
        assert!(!anchor::is_unanchored(h));
        assert_eq!(h.end - h.start, quote.chars().count() as i32);
        let mut gone = hl("g", -1, -1, "this text is not in the book, not even a bit");
        gone.note = "n".into();
        lib.get_mut(id).unwrap().highlights.push(gone);
        assert!(!relocate_one(&mut lib, id, "g"));
        let _ = std::fs::remove_dir_all(d);
    }

    /// Every way a highlight reaches the library (Kobo database, KOReader notes file, Kindle
    /// clippings, a JSON export) is anchored by its text: whatever offsets or paths the device
    /// wrote, the stored range holds the quote in the reader's text.
    #[test]
    fn every_import_style_stores_the_quote_where_the_reader_has_it() {
        let d = tmp();
        let (mut lib, id, quote) = lib_with_book(&d, 40);
        let books = crate::device_sync::library_books(&lib.books);
        let locator = crate::device_sync::book_locator(&lib);

        // Kobo: the Text column; its container path and offsets (here made up like the device's
        // span ids) belong to the device's own rendering and are not used
        let kobo = d.join("KoboReader.sqlite");
        bookx_serve::kobo::create_real_schema_database(&kobo).unwrap();
        bookx_serve::kobo::add_test_highlight(
            &kobo,
            "file:///mnt/onboard/m.epub",
            "Micro",
            "Sam",
            &quote.replace("it, and", "it and").replace('\'', "’"),
            "kobo note",
        )
        .unwrap();
        // KOReader: xpointers into crengine's DOM next to the text
        let lua = String::from(
            "return {\n [\"annotations\"] = {\n  [1] = { [\"text\"] = \"Paragraph number 31 has some words in it.\", [\"pos0\"] = \"/body/DocFragment[1]/body/p[33]/text().0\", [\"pos1\"] = \"/body/DocFragment[1]/body/p[33]/text().40\", [\"datetime\"] = \"2026-08-17 11:22:15\" },\n },\n [\"doc_props\"] = { [\"title\"] = \"Micro\", [\"authors\"] = \"Sam\" },\n}\n"
        );
        let sdr = d.join("Micro.sdr");
        std::fs::create_dir_all(&sdr).unwrap();
        std::fs::write(sdr.join("metadata.epub.lua"), lua).unwrap();
        // Kindle: whitespace is collapsed and quotes are straight
        let clip = "\u{feff}Micro (Sam)\r\n- Your Highlight on page 3 | Location 30-31 | Added on Monday, 3 March 2014 10:42:11\r\n\r\nParagraph   number 7\nhas some words in it.\r\n==========\r\n";
        std::fs::write(d.join("My Clippings.txt"), clip).unwrap();
        // a JSON export
        let json = r#"{"title":"Micro","author":"Sam","entries":[{"page":3,"time":1700000000,"text":"PARAGRAPH NUMBER 12 HAS SOME WORDS IN IT.","note":"shout"}],"version":"1.0"}"#;
        std::fs::write(d.join("ko.json"), json).unwrap();

        let paths = vec![
            kobo.clone(),
            sdr.join("metadata.epub.lua"),
            d.join("My Clippings.txt"),
            d.join("ko.json"),
        ];
        let (items, problems) = crate::annotation_import::read_paths(&paths);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(items.len(), 4);
        let plan = crate::annotation_import::plan(&items, &books, &locator);
        assert_eq!(plan.unplaced(), 0, "{}", plan.text());
        crate::annotation_import::apply(&mut lib, &plan);

        let open = lib.open_book(id).unwrap();
        let doc = open.chapter_doc(&open.spine_paths()[0]).unwrap();
        let rt: Vec<char> = doc.reader_text().chars().collect();
        let hs = &lib.get(id).unwrap().highlights;
        assert_eq!(hs.len(), 4);
        for h in hs {
            assert!(!anchor::is_unanchored(h), "{}", h.text);
            let on_page: String = rt[h.start as usize..h.end as usize].iter().collect();
            // the Kobo text lost a comma: found by similarity, the whole passage is still the range
            let fuzzy = h.text.contains("deploy it and release");
            assert!(
                anchor::same_text(&on_page, &h.text) || fuzzy,
                "stored range holds {on_page:?} but the highlight says {:?}",
                h.text
            );
            // and it is the place the paragraph really has (counted line by line)
            let line = doc
                .reader_text()
                .split('\n')
                .position(|l| l == on_page.trim())
                .unwrap();
            let before: usize = doc
                .reader_text()
                .split('\n')
                .take(line)
                .map(|l| l.chars().count() + 1)
                .sum();
            assert_eq!(h.start as usize, before, "{}", h.text);
        }
        // a second run repairs nothing: they are all where their text is
        let r = lib.repair_highlights(None).unwrap();
        assert_eq!((r.moved, r.unanchored, r.fine), (0, 0, 4));
        let _ = std::fs::remove_dir_all(d);
    }

    /// Bookx → device: the notes file names the chapter and carries the exact text; it never
    /// claims a position inside the chapter, so nothing can be shown at a wrong place there, and
    /// reading it back finds every highlight again by its text (also one without a place).
    #[test]
    fn what_is_written_for_koreader_reads_back_as_the_same_highlights() {
        let d = tmp();
        let (lib, id, quote) = lib_with_book(&d, 5);
        let mut lost = hl(
            "b",
            -1,
            -1,
            "A “curly” quote\nwith a line break and \"double\" marks.",
        );
        lost.note = "my \"note\"".into();
        let sent = vec![hl("a", 40, 40 + quote.chars().count() as i32, &quote), lost];
        let notes = crate::send_notes::Notes::new(&sent, &[]);
        let lua = crate::send_notes::koreader_sidecar("Micro", &["One".to_string()], &notes);
        let back = crate::annotation_import::read_koreader_lua(
            &lua,
            Path::new("Micro.sdr/metadata.epub.lua"),
        );
        assert_eq!(back.len(), 2);
        for h in &sent {
            assert!(
                back.iter().any(|b| b.text == h.text.trim()),
                "text of {:?} survives: {back:?}",
                h.id
            );
        }
        // the book's own copy of them is "already there": nothing is added twice
        let mut lib = lib;
        lib.get_mut(id).unwrap().highlights = sent.clone();
        let books = crate::device_sync::library_books(&lib.books);
        let plan =
            crate::annotation_import::plan(&back, &books, &crate::device_sync::book_locator(&lib));
        assert_eq!(plan.new_highlights(), 0, "{}", plan.text());
        assert_eq!(plan.already(), 2);
        let _ = std::fs::remove_dir_all(d);
    }
}
