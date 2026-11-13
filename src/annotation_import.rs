//! Importing highlights and notes made elsewhere into the library (Kindle `My Clippings.txt`,
//! KOReader sidecars and exports, a Kobo database, a calibre annotation collection, Bookx's own
//! exports).
//!
//! The rule is the same as for the device sync: **additions only**. A quote the library already has
//! (same text ignoring case and punctuation, or one inside the other) is never added again; its
//! note is only filled in when the library's is empty; nothing is deleted or changed otherwise.
//! The work is three pure steps: [`read_path`] (files to [`Imported`]), [`plan`] (what would be
//! added, book by book, shown to the user) and [`apply`] (which reuses the device sync's
//! `apply_book`, so the two paths cannot disagree).

use crate::i18n::tr;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bookx_lookup::import::{self, Imported};
use bookx_serve::syncplan::{self, Annot, Pairing};
use chrono::{DateTime, Utc};

use crate::device_sync::{self, BookPlan, DeviceNote, Incoming, LibraryBook, Locator};

/// Reads whatever `path` is: a file of a known kind, or a folder (a device root, a KOReader
/// folder) searched for the files below. Files are only read, never written.
pub fn read_path(path: &Path) -> Result<Vec<Imported>> {
    if path.is_dir() {
        return read_folder(path);
    }
    read_file(path)
}

fn is_sqlite(bytes: &[u8]) -> bool {
    bytes.starts_with(b"SQLite format 3\0")
}

/// One file, recognised by its content first and its name second.
pub fn read_file(path: &Path) -> Result<Vec<Imported>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if is_sqlite(&bytes) {
        return read_kobo(path);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return import::parse_json(&text);
    }
    if name.ends_with(".lua") || trimmed.starts_with("return {") || trimmed.starts_with("-- ") {
        return Ok(read_koreader_lua(&text, path));
    }
    if text.contains("==========") {
        return Ok(import::dedupe(import::parse_kindle_clippings(&text)));
    }
    if trimmed.starts_with("# ") {
        return Ok(import::parse_koreader_markdown(&text));
    }
    bail!(
        "{}: not a file Bookx can read highlights from (Kindle My Clippings.txt, KOReader notes or export, Kobo database, calibre or Bookx annotation file)",
        path.display()
    )
}

/// The Kobo `Bookmark` table of a device or of a copy of its `KoboReader.sqlite`.
pub fn read_kobo(db: &Path) -> Result<Vec<Imported>> {
    Ok(bookx_serve::kobo::read_book_notes(db)?
        .into_iter()
        .map(|b| imported_of(&b.note, b.title, b.author))
        .collect())
}

fn imported_of(
    n: &bookx_serve::reading::DeviceNote,
    title: String,
    author: Option<String>,
) -> Imported {
    let when = n
        .created
        .as_deref()
        .and_then(bookx_serve::reading::parse_time)
        .and_then(|t| DateTime::from_timestamp(t, 0));
    let mut note = n.note.trim().to_string();
    if note == n.text.trim() {
        // KOReader's older files repeat the quote in `notes` when there is no note
        note.clear();
    }
    Imported {
        book_title: title,
        book_author: author,
        text: n.text.trim().to_string(),
        note,
        location: None,
        chapter: n.chapter.clone(),
        created: when,
        bookmark: false,
    }
}

/// A KOReader `metadata.*.lua`: the title comes from `doc_props`, else from the `.sdr` folder.
pub fn read_koreader_lua(text: &str, path: &Path) -> Vec<Imported> {
    let side = bookx_serve::reading::parse_sidecar(text);
    let root = bookx_serve::lua::parse(text);
    let props = root.as_ref().and_then(|r| r.get("doc_props"));
    let title = props
        .and_then(|p| p.str("title"))
        .map(str::to_string)
        .filter(|t| !t.trim().is_empty())
        .or_else(|| {
            path.parent()
                .and_then(|d| d.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .and_then(|n| n.strip_suffix(".sdr").map(str::to_string))
        })
        .unwrap_or_default();
    let author = props
        .and_then(|p| p.str("authors"))
        .map(|a| a.replace('\n', "; "))
        .filter(|a| !a.trim().is_empty());
    if title.is_empty() {
        return Vec::new();
    }
    side.notes
        .iter()
        .map(|n| imported_of(n, title.clone(), author.clone()))
        .filter(|i| !i.text.is_empty() || !i.note.is_empty())
        .collect()
}

/// Looks through a folder (a device's root, a KOReader data folder) for a Kobo database, Kindle
/// clippings and KOReader sidecars. Only reads.
pub fn read_folder(dir: &Path) -> Result<Vec<Imported>> {
    let mut out = Vec::new();
    if let Some(db) = bookx_serve::devices::kobo_db_path(dir) {
        out.extend(read_kobo(&db)?);
    }
    for rel in ["documents/My Clippings.txt", "My Clippings.txt"] {
        let p = dir.join(rel);
        if p.is_file() {
            out.extend(read_file(&p)?);
        }
    }
    let mut budget = 20_000usize;
    sidecars_in(dir, 0, &mut budget, &mut out);
    Ok(out)
}

fn sidecars_in(dir: &Path, depth: usize, budget: &mut usize, out: &mut Vec<Imported>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if depth < 6 && !name.starts_with(".kobo") {
                sidecars_in(&p, depth + 1, budget, out);
            }
        } else if name.starts_with("metadata.") && name.ends_with(".lua") {
            if let Ok(text) = std::fs::read_to_string(&p) {
                out.extend(read_koreader_lua(&text, &p));
            }
        }
    }
}

/// Reads several paths; a path that cannot be read is reported and does not stop the others.
pub fn read_paths(paths: &[PathBuf]) -> (Vec<Imported>, Vec<String>) {
    let (mut all, mut problems) = (Vec::new(), Vec::new());
    for p in paths {
        match read_path(p) {
            Ok(v) => all.extend(v),
            Err(e) => problems.push(format!("{e:#}")),
        }
    }
    (import::dedupe(all), problems)
}

// -- planning ---------------------------------------------------------------------------------

/// What importing does to one library book.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BookImport {
    /// `highlights_in`, `notes_in` and `unplaced` of the device sync's book plan are the new
    /// highlights, the notes added to highlights the book had without one, and the quotes that
    /// could not be found in the book's text.
    pub plan: BookPlan,
    /// Quotes the library already had.
    pub already: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportPlan {
    pub books: Vec<BookImport>,
    /// Books of the file that are not in the library: `(title, how many notes)`.
    pub unmatched: Vec<(String, usize)>,
    /// Books that fit more than one library book: `(title, how many notes)`.
    pub ambiguous: Vec<(String, usize)>,
    /// Bookmarks and notes without a quote, which have no place in the book to attach to.
    pub skipped: usize,
}

impl ImportPlan {
    pub fn new_highlights(&self) -> usize {
        self.books.iter().map(|b| b.plan.highlights_in.len()).sum()
    }

    pub fn notes_added(&self) -> usize {
        self.books.iter().map(|b| b.plan.notes_in.len()).sum()
    }

    pub fn already(&self) -> usize {
        self.books.iter().map(|b| b.already).sum()
    }

    pub fn unplaced(&self) -> usize {
        self.books.iter().map(|b| b.plan.unplaced).sum()
    }

    /// Whether applying changes anything.
    pub fn is_empty(&self) -> bool {
        self.new_highlights() + self.notes_added() == 0
    }

    /// The review text: per book what is added, then what is left out and why.
    pub fn lines(&self) -> Vec<String> {
        let mut v = Vec::new();
        let n =
            |c: usize, one: &str, many: &str| format!("{c} {}", if c == 1 { one } else { many });
        for b in &self.books {
            let p = &b.plan;
            let mut parts = Vec::new();
            if !p.highlights_in.is_empty() {
                parts.push(format!(
                    "{} added",
                    n(p.highlights_in.len(), "highlight", "highlights")
                ));
            }
            if !p.notes_in.is_empty() {
                parts.push(format!(
                    "{} filled in",
                    n(p.notes_in.len(), "empty note", "empty notes")
                ));
            }
            if b.already > 0 {
                parts.push(format!("{} already there", b.already));
            }
            if p.unplaced > 0 {
                parts.push(format!(
                    "{} not found in the book’s text, left out",
                    p.unplaced
                ));
            }
            if p.highlights_in.is_empty()
                && p.notes_in.is_empty()
                && b.already == 0
                && p.unplaced == 0
            {
                continue;
            }
            v.push(format!("{}: {}", p.title, parts.join(", ")));
        }
        if v.is_empty() {
            v.push(tr("Nothing to add."));
        }
        for (what, list) in [
            ("not in your library", &self.unmatched),
            ("match more than one book in your library", &self.ambiguous),
        ] {
            if !list.is_empty() {
                let shown: Vec<String> = list
                    .iter()
                    .take(5)
                    .map(|(t, c)| format!("{t} ({c})"))
                    .collect();
                let more = list.len().saturating_sub(5);
                v.push(format!(
                    "Left out, {what}: {}{}",
                    shown.join(", "),
                    if more > 0 {
                        format!(" and {more} more")
                    } else {
                        String::new()
                    }
                ));
            }
        }
        if self.skipped > 0 {
            v.push(format!(
                "{} left out: bookmarks and notes without a highlighted quote",
                self.skipped
            ));
        }
        v.push(
            "Nothing is deleted, nothing you already have is changed, no quote is added twice."
                .to_string(),
        );
        v
    }

    pub fn text(&self) -> String {
        self.lines().join("\n")
    }
}

fn annot_of(h: &crate::core::library::Highlight) -> Annot {
    Annot {
        text: h.text.clone(),
        note: h.note.clone(),
        when: None,
    }
}

/// Works out what importing `items` into `library` would do. Changes nothing.
pub fn plan(items: &[Imported], library: &[LibraryBook], locator: &dyn Locator) -> ImportPlan {
    let sides: Vec<_> = library.iter().map(device_sync::side_of_library).collect();
    let mut out = ImportPlan::default();
    let mut by_book: BTreeMap<u64, BookImport> = BTreeMap::new();
    let mut unmatched: BTreeMap<String, usize> = BTreeMap::new();
    let mut ambiguous: BTreeMap<String, usize> = BTreeMap::new();
    for it in import::dedupe(items.to_vec()) {
        if it.bookmark || it.text.trim().is_empty() {
            out.skipped += 1;
            continue;
        }
        let authors = it
            .book_author
            .as_deref()
            .map(import::split_authors)
            .unwrap_or_default();
        let idx = match syncplan::pair(&sides, &it.book_title, &authors) {
            Pairing::One(i) => i,
            Pairing::NoBook => {
                *unmatched.entry(it.book_title.clone()).or_default() += 1;
                continue;
            }
            Pairing::Ambiguous => {
                *ambiguous.entry(it.book_title.clone()).or_default() += 1;
                continue;
            }
        };
        let book = &library[idx];
        let bi = by_book.entry(book.id).or_insert_with(|| BookImport {
            plan: BookPlan {
                book_id: book.id,
                title: book.title.clone(),
                ..Default::default()
            },
            already: 0,
        });
        let want = Annot {
            text: it.text.clone(),
            note: it.note.clone(),
            when: None,
        };
        match book
            .highlights
            .iter()
            .find(|h| syncplan::same_highlight(&annot_of(h), &want))
        {
            Some(h) => {
                bi.already += 1;
                if h.note.trim().is_empty()
                    && !it.note.trim().is_empty()
                    && !bi.plan.notes_in.iter().any(|(id, _)| *id == h.id)
                {
                    bi.plan
                        .notes_in
                        .push((h.id.clone(), it.note.trim().to_string()));
                }
            }
            None => match locator.locate(book.id, &it.text) {
                Some((spine, start, end)) => bi.plan.highlights_in.push(Incoming {
                    note: DeviceNote {
                        id: None,
                        text: it.text.clone(),
                        note: it.note.clone(),
                        created: it.created,
                    },
                    spine,
                    start,
                    end,
                }),
                None => bi.plan.unplaced += 1,
            },
        }
    }
    out.books = by_book.into_values().collect();
    out.unmatched = unmatched.into_iter().collect();
    out.ambiguous = ambiguous.into_iter().collect();
    out
}

/// What [`apply`] added.
pub use crate::device_sync::Applied;

/// Adds the planned highlights and notes to the library (progress is not touched).
pub fn apply(lib: &mut crate::core::library::Library, plan: &ImportPlan) -> Applied {
    let now = Utc::now();
    let mut total = Applied::default();
    for b in &plan.books {
        if let Some(book) = lib.get_mut(b.plan.book_id) {
            let a = device_sync::apply_book(book, &b.plan, now);
            total.highlights += a.highlights;
            total.notes += a.notes;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::Highlight;
    use chrono::TimeZone;

    struct Anywhere;
    impl Locator for Anywhere {
        fn locate(&self, _: u64, text: &str) -> Option<(usize, i32, i32)> {
            (!text.contains("MISSING")).then_some((1, 10, 10 + text.len() as i32))
        }
    }

    fn imp(title: &str, author: Option<&str>, text: &str, note: &str) -> Imported {
        Imported {
            book_title: title.into(),
            book_author: author.map(str::to_string),
            text: text.into(),
            note: note.into(),
            location: None,
            chapter: None,
            created: Some(Utc.with_ymd_and_hms(2024, 3, 3, 10, 42, 11).unwrap()),
            bookmark: false,
        }
    }

    fn lib_book(id: u64, title: &str, author: &str, hs: Vec<Highlight>) -> LibraryBook {
        LibraryBook {
            id,
            title: title.into(),
            authors: vec![author.into()],
            progress: 0.0,
            last_read: None,
            highlights: hs,
        }
    }

    fn hl(id: &str, text: &str, note: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start: 0,
            end: 1,
            color: "yellow".into(),
            style: String::new(),
            note: note.into(),
            text: text.into(),
            created: Utc::now(),
        }
    }

    #[test]
    fn new_quotes_are_added_known_ones_are_not_and_empty_notes_are_filled() {
        let lib = [lib_book(
            1,
            "Dune",
            "Frank Herbert",
            vec![
                hl("a", "Fear is the mind-killer.", ""),
                hl("b", "Keep this one.", "mine"),
            ],
        )];
        let items = [
            imp(
                "Dune",
                Some("Frank Herbert"),
                "fear is the MIND killer",
                "late note",
            ),
            imp("Dune", Some("Frank Herbert"), "keep this one", "theirs"),
            imp(
                "Dune",
                Some("Frank Herbert"),
                "A brand new quote here.",
                "n",
            ),
            imp("Dune", Some("Frank Herbert"), "MISSING from the text", ""),
            imp("Not Mine", None, "x quote", ""),
        ];
        let p = plan(&items, &lib, &Anywhere);
        assert_eq!(p.new_highlights(), 1);
        assert_eq!(p.notes_added(), 1);
        assert_eq!(p.already(), 2);
        assert_eq!(p.unplaced(), 1);
        assert_eq!(p.unmatched, vec![("Not Mine".to_string(), 1)]);
        let text = p.text();
        assert!(
            text.contains(
                "Dune: 1 highlight added, 1 empty note filled in, 2 already there, 1 not found"
            ),
            "{text}"
        );
        assert!(text.contains("Left out, not in your library: Not Mine (1)"));
        assert!(text.ends_with("no quote is added twice."));
    }

    #[test]
    fn applying_adds_once_never_duplicates_never_overwrites() {
        let lib0 = vec![
            hl("a", "Fear is the mind-killer.", ""),
            hl("b", "Keep this one.", "mine"),
        ];
        let lb = [lib_book(1, "Dune", "Frank Herbert", lib0.clone())];
        let items = [
            imp("Dune", None, "Fear is the mind-killer", "late note"),
            imp("Dune", None, "Keep this one.", "theirs"),
            imp("Dune", None, "A brand new quote here.", "n"),
        ];
        let p = plan(&items, &lb, &Anywhere);
        let dir = std::env::temp_dir().join(format!("bookx-ai-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut lib = bookx_store::open_library(&dir.join("lib")).unwrap().library;
        let src = dir.join("b.epub");
        let md = crate::core::opf::Metadata {
            title: "Dune".into(),
            authors: vec!["Frank Herbert".into()],
            ..Default::default()
        };
        let ch = crate::core::convert::Chapter {
            title: "One".into(),
            body: "<p>Words.</p>".into(),
        };
        std::fs::write(
            &src,
            crate::core::convert::create_epub(&md, &[ch], None)
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        lib.add_path(&src, true).unwrap();
        let id = lib.books[0].id;
        lib.get_mut(id).unwrap().highlights = lib0;
        let mut p = p;
        for b in &mut p.books {
            b.plan.book_id = id;
        }
        let a = apply(&mut lib, &p);
        assert_eq!((a.highlights, a.notes), (1, 1));
        let hs = &lib.get(id).unwrap().highlights;
        assert_eq!(hs.len(), 3);
        assert_eq!(hs[0].note, "late note");
        assert_eq!(hs[1].note, "mine", "an existing note is never overwritten");
        assert_eq!(hs[2].text, "A brand new quote here.");
        assert_eq!(
            hs[2].created,
            Utc.with_ymd_and_hms(2024, 3, 3, 10, 42, 11).unwrap()
        );
        let again = apply(&mut lib, &p);
        assert_eq!((again.highlights, again.notes), (0, 0));
        assert_eq!(lib.get(id).unwrap().highlights.len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_second_plan_after_applying_has_nothing_to_add() {
        let mut hs = vec![];
        let items = [imp("Dune", None, "A brand new quote here.", "n")];
        let p = plan(&items, &[lib_book(1, "Dune", "F", hs.clone())], &Anywhere);
        assert_eq!(p.new_highlights(), 1);
        hs.push(hl("x", "A brand new quote here.", "n"));
        let p2 = plan(&items, &[lib_book(1, "Dune", "F", hs)], &Anywhere);
        assert!(p2.is_empty());
        assert_eq!(p2.already(), 1);
    }

    #[test]
    fn the_same_title_by_another_author_and_two_matching_books_are_not_guessed() {
        let lib = [
            lib_book(1, "Dune", "Frank Herbert", vec![]),
            lib_book(2, "Dune", "Someone Else", vec![]),
        ];
        let p = plan(&[imp("Dune", None, "quote one here", "")], &lib, &Anywhere);
        assert_eq!(p.ambiguous.len(), 1);
        assert!(p.is_empty());
        let p = plan(
            &[imp("Dune", Some("Herbert, Frank"), "quote one here", "")],
            &lib,
            &Anywhere,
        );
        assert_eq!(p.new_highlights(), 1);
        assert_eq!(p.books[0].plan.book_id, 1);
        let only = [lib_book(1, "Dune", "Frank Herbert", vec![])];
        let p = plan(
            &[imp("Dune", Some("Jane Austen"), "quote one here", "")],
            &only,
            &Anywhere,
        );
        assert_eq!(p.unmatched.len(), 1);
    }

    #[test]
    fn bookmarks_and_quoteless_notes_are_counted_not_imported() {
        let lib = [lib_book(1, "Dune", "F", vec![])];
        let mut bm = imp("Dune", None, "", "");
        bm.bookmark = true;
        let p = plan(
            &[bm, imp("Dune", None, "", "a loose note")],
            &lib,
            &Anywhere,
        );
        assert_eq!(p.skipped, 2);
        assert!(p.text().contains("2 left out: bookmarks and notes without"));
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-ai-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn files_are_recognised_by_content() {
        let d = tmp();
        std::fs::write(
            d.join("My Clippings.txt"),
            "Dune (Frank Herbert)\n- Your Highlight on Location 5-6 | Added on Monday, 3 March 2014 10:42:11\n\nFear is the mind-killer.\n==========\n",
        )
        .unwrap();
        assert_eq!(read_path(&d.join("My Clippings.txt")).unwrap().len(), 1);
        std::fs::write(
            d.join("e.json"),
            r#"{"title":"Dune","author":"F","entries":[{"text":"one quote here","page":1,"time":1700000000}]}"#,
        )
        .unwrap();
        assert_eq!(
            read_path(&d.join("e.json")).unwrap()[0].text,
            "one quote here"
        );
        std::fs::write(
            d.join("e.md"),
            "# Dune\n### Page 1 @ 03 March 2024 10:42:11 AM\nquote text\n",
        )
        .unwrap();
        assert_eq!(read_path(&d.join("e.md")).unwrap().len(), 1);
        std::fs::write(d.join("x.bin"), "nothing useful").unwrap();
        assert!(read_path(&d.join("x.bin")).is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_koreader_sidecar_and_a_folder_of_them_are_read() {
        let d = tmp();
        let sdr = d.join("Books/Dune.sdr");
        std::fs::create_dir_all(&sdr).unwrap();
        let lua = "return {\n [\"annotations\"] = {\n  [1] = { [\"text\"] = \"Fear is the mind-killer.\", [\"note\"] = \"litany\", [\"datetime\"] = \"2026-08-17 11:22:15\" },\n  [2] = { [\"text\"] = \"Quote two here\", [\"datetime\"] = \"2026-08-17 11:23:15\" },\n },\n [\"doc_props\"] = { [\"title\"] = \"Dune\", [\"authors\"] = \"Frank Herbert\" },\n [\"percent_finished\"] = 0.6,\n}\n";
        std::fs::write(sdr.join("metadata.epub.lua"), lua).unwrap();
        let one = read_path(&sdr.join("metadata.epub.lua")).unwrap();
        assert_eq!(one.len(), 2);
        assert_eq!(one[0].book_title, "Dune");
        assert_eq!(one[0].book_author.as_deref(), Some("Frank Herbert"));
        assert_eq!(one[0].note, "litany");
        assert_eq!(
            one[0].created,
            Some(Utc.with_ymd_and_hms(2026, 8, 17, 11, 22, 15).unwrap())
        );
        assert_eq!(read_path(&d).unwrap().len(), 2, "a folder is searched");
        // without doc_props the .sdr folder names the book
        std::fs::write(
            sdr.join("metadata.epub.lua"),
            "return { [\"annotations\"] = { [1] = { [\"text\"] = \"A quote there\" } } }",
        )
        .unwrap();
        assert_eq!(
            read_path(&sdr.join("metadata.epub.lua")).unwrap()[0].book_title,
            "Dune"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_mounted_kindle_gives_its_clippings_and_nothing_is_written() {
        let d = tmp();
        let docs = d.join("documents");
        std::fs::create_dir_all(docs.join("Dune.sdr")).unwrap();
        std::fs::create_dir_all(d.join("system")).unwrap();
        let clippings = "\u{feff}Dune (Frank Herbert)\r\n- Your Highlight on page 3 | Location 30-31 | Added on Monday, 3 March 2014 10:42:11\r\n\r\nFear is the mind-killer.\r\n==========\r\n";
        std::fs::write(docs.join("My Clippings.txt"), clippings).unwrap();
        // the Kindle's own per-book files are binary and not read: they must neither break the
        // import nor be touched
        std::fs::write(docs.join("Dune.sdr/Dune.azw3r"), [0u8, 159, 1, 2, 3]).unwrap();
        std::fs::write(docs.join("Dune.sdr/Dune.mbp"), [255u8, 0, 7]).unwrap();
        let before: Vec<_> = [
            "My Clippings.txt",
            "Dune.sdr/Dune.azw3r",
            "Dune.sdr/Dune.mbp",
        ]
        .iter()
        .map(|f| std::fs::read(docs.join(f)).unwrap())
        .collect();
        let (items, problems) = read_paths(std::slice::from_ref(&d));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].book_title, "Dune");
        assert_eq!(items[0].text, "Fear is the mind-killer.");
        let after: Vec<_> = [
            "My Clippings.txt",
            "Dune.sdr/Dune.azw3r",
            "Dune.sdr/Dune.mbp",
        ]
        .iter()
        .map(|f| std::fs::read(docs.join(f)).unwrap())
        .collect();
        assert_eq!(before, after, "read only");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_copied_kobo_database_is_read_without_touching_it() {
        let d = tmp();
        let db = d.join("KoboReader.sqlite");
        bookx_serve::kobo::create_real_schema_database(&db).unwrap();
        bookx_serve::kobo::add_test_highlight(
            &db,
            "file:///mnt/onboard/Dune.epub",
            "Dune",
            "Frank Herbert",
            "Fear is the mind-killer.",
            "litany",
        )
        .unwrap();
        let before = std::fs::read(&db).unwrap();
        let v = read_path(&db).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(
            (v[0].book_title.as_str(), v[0].note.as_str()),
            ("Dune", "litany")
        );
        assert_eq!(v[0].book_author.as_deref(), Some("Frank Herbert"));
        assert_eq!(std::fs::read(&db).unwrap(), before);
        // a device root with the database in .kobo is found too
        std::fs::create_dir_all(d.join("dev/.kobo")).unwrap();
        std::fs::copy(&db, d.join("dev/.kobo/KoboReader.sqlite")).unwrap();
        assert_eq!(read_path(&d.join("dev")).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }
}
