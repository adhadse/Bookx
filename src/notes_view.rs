//! Glue for the *Notes & Highlights* window: the library's highlights as `bookx_lookup::notes_query`
//! rows, editing and deleting one, and turning a selection into export items, Anki cards and
//! Readwise batches. No GTK in here, so it is unit tested; the window is `ui::notes`.

use std::collections::HashMap;

use bookx_lookup::anki::{self, CardKind, Options as AnkiOptions, Separator};
use bookx_lookup::export::AnnotationItem;
use bookx_lookup::notes_query::{NoteRow, RowKind};
use chrono::{DateTime, Utc};

use crate::core::library::{Book, Library};

/// Every highlight and bookmark of `books` as a row (a book's highlights, then its bookmarks). A PDF
/// highlight carries its page, others the chapter number (titles are only looked up for exports,
/// they need the book opened). A bookmark row has its label as its text and its place as a
/// chapter and how far into it.
pub fn rows_of(books: &[Book]) -> Vec<NoteRow> {
    let mut out = Vec::new();
    for b in books {
        let author = Some(b.meta.authors_display()).filter(|a| !a.trim().is_empty());
        for h in &b.highlights {
            let pdf = crate::pdf::is_pdf(h);
            out.push(NoteRow {
                kind: RowKind::Highlight,
                frac: 0.0,
                book_id: b.id,
                highlight_id: h.id.clone(),
                book_title: b.meta.title.clone(),
                book_author: author.clone(),
                book_tags: b.meta.tags.clone(),
                spine: h.spine,
                chapter: None,
                text: h.text.clone(),
                note: h.note.clone(),
                color: h.color.clone(),
                style: crate::annotation_export::style_of(&h.style),
                created: h.created,
                page: pdf.then_some(h.spine + 1),
                unplaced: !pdf && crate::core::anchor::is_unanchored(h),
            });
        }
        for m in &b.bookmarks {
            out.push(NoteRow {
                kind: RowKind::Bookmark,
                frac: m.frac,
                book_id: b.id,
                highlight_id: m.id.clone(),
                book_title: b.meta.title.clone(),
                book_author: author.clone(),
                book_tags: b.meta.tags.clone(),
                spine: m.spine,
                chapter: None,
                text: m.label.clone(),
                note: String::new(),
                color: String::new(),
                style: bookx_lookup::export::HighlightStyle::Bookmark,
                created: m.created,
                // a PDF bookmark is a page
                page: b
                    .file
                    .to_lowercase()
                    .ends_with(".pdf")
                    .then_some(m.spine + 1),
                unplaced: false,
            });
        }
    }
    out
}

pub fn rows(lib: &Library) -> Vec<NoteRow> {
    rows_of(&lib.books)
}

/// Changes the note of one highlight. False if it is not there (deleted meanwhile).
pub fn set_note(books: &mut [Book], book_id: u64, highlight_id: &str, note: &str) -> bool {
    books
        .iter_mut()
        .find(|b| b.id == book_id)
        .and_then(|b| b.highlights.iter_mut().find(|h| h.id == highlight_id))
        .map(|h| h.note = note.trim().to_string())
        .is_some()
}

/// Changes the colour and/or the style of one highlight (not the PDF ones, whose style holds
/// the marked rectangles). False if it is not there or cannot be changed.
pub fn set_look(
    books: &mut [Book],
    book_id: u64,
    highlight_id: &str,
    color: Option<&str>,
    style: Option<&str>,
) -> bool {
    let Some(h) = books
        .iter_mut()
        .find(|b| b.id == book_id)
        .and_then(|b| b.highlights.iter_mut().find(|h| h.id == highlight_id))
    else {
        return false;
    };
    if h.style.starts_with("pdf|") {
        return false;
    }
    if let Some(c) = color {
        h.color = c.to_string();
    }
    if let Some(s) = style {
        h.style = s.to_string();
    }
    true
}

/// Removes one highlight. False if it is not there.
pub fn delete(books: &mut [Book], book_id: u64, highlight_id: &str) -> bool {
    match books.iter_mut().find(|b| b.id == book_id) {
        Some(b) => {
            let n = b.highlights.len();
            b.highlights.retain(|h| h.id != highlight_id);
            b.highlights.len() != n
        }
        None => false,
    }
}

/// The line under a bookmark's title in the reader's sidebar and in the Notes list: the label, the
/// day, where it came from (`KOReader (Kobo Libra)`) and, for a rough place, that it is rough.
/// `from` is `(source, approximate)` of a bookmark a device brought.
pub fn bookmark_line(label: &str, created: DateTime<Utc>, from: Option<&(String, bool)>) -> String {
    let mut s = format!("{label} · {}", created.format("%d %b %Y"));
    if let Some((src, approx)) = from {
        if !src.is_empty() {
            s.push_str(&format!(" · {src}"));
        }
        if *approx {
            s.push_str(&format!(" · {}", crate::i18n::tr("approximate place")));
        }
    }
    s
}

/// Removes one bookmark (what [`delete`] does for a highlight). The sync's ledger keeps its entry,
/// so a bookmark that came from a device is not brought back by the next sync. `None` when it is
/// not there.
pub fn delete_bookmark(
    books: &mut [Book],
    book_id: u64,
    bookmark_id: &str,
) -> Option<crate::remove_undo::Removed> {
    let b = books.iter_mut().find(|b| b.id == book_id)?;
    crate::remove_undo::take_bookmark(&mut b.bookmarks, bookmark_id)
}

/// Export items for `rows` (chapter titles filled in for EPUB highlights, which opens each book
/// once). Order: as given.
pub fn items_for(lib: &Library, rows: &[&NoteRow]) -> Vec<AnnotationItem> {
    let mut titles: HashMap<u64, Vec<String>> = HashMap::new();
    rows.iter()
        .map(|r| {
            let mut r = (*r).clone();
            if r.page.is_none() {
                let t = titles
                    .entry(r.book_id)
                    .or_insert_with(|| crate::ui::annotations::chapter_titles(lib, r.book_id));
                r.chapter = t.get(r.spine).cloned();
            }
            r.to_item()
        })
        .collect()
}

/// The file name for an export of `n` highlights.
pub fn file_name(id: &str, n_books: usize, only_title: Option<&str>) -> String {
    match (n_books, only_title) {
        (1, Some(t)) => crate::annotation_export::file_name(t, id),
        _ => {
            let ext = crate::annotation_export::format(id).map_or("txt", |f| f.extension);
            format!("Notes and highlights.{ext}")
        }
    }
}

/// The Anki choices of the export menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnkiFile {
    Apkg,
    Tsv,
    Csv,
}

impl AnkiFile {
    pub fn parse(s: &str) -> Option<(AnkiFile, CardKind)> {
        let (f, k) = s.split_once('-')?;
        let file = match f {
            "apkg" => AnkiFile::Apkg,
            "tsv" => AnkiFile::Tsv,
            "csv" => AnkiFile::Csv,
            _ => return None,
        };
        let kind = match k {
            "basic" => CardKind::Basic,
            "cloze" => CardKind::Cloze,
            _ => return None,
        };
        Some((file, kind))
    }

    pub fn extension(self) -> &'static str {
        match self {
            AnkiFile::Apkg => "apkg",
            AnkiFile::Tsv => "txt",
            AnkiFile::Csv => "csv",
        }
    }
}

/// The bytes of an Anki file for `items` (highlights) or, with `vocab`, for the vocabulary list.
pub fn anki_bytes(
    file: AnkiFile,
    kind: CardKind,
    deck: &str,
    items: &[AnnotationItem],
    vocab: Option<(
        &bookx_lookup::vocab::VocabStore,
        &dyn Fn(&str) -> Option<String>,
    )>,
    now: DateTime<Utc>,
) -> anyhow::Result<(Vec<u8>, usize)> {
    let opts = AnkiOptions {
        deck: deck.to_string(),
        kind,
        tags: Vec::new(),
    };
    let notes = match vocab {
        Some((store, title)) => anki::notes_from_vocab(store, &opts, title),
        None => anki::notes_from_items(items, &opts),
    };
    let n = notes.len();
    let bytes = match file {
        AnkiFile::Apkg => anki::to_apkg(&notes, deck, now)?,
        AnkiFile::Tsv => anki::to_delimited(&notes, &opts, Separator::Tab).into_bytes(),
        AnkiFile::Csv => anki::to_delimited(&notes, &opts, Separator::Comma).into_bytes(),
    };
    Ok((bytes, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::Highlight;
    use chrono::TimeZone;

    fn book(id: u64, title: &str, hs: &[(&str, &str, &str)]) -> Book {
        let mut b: Book = serde_json::from_value(serde_json::json!({
            "id": id, "file": "a.epub",
            "meta": {"title": title, "title_sort": "", "authors": ["An Author"], "author_sort": "", "publisher": "", "language": "", "description": "", "tags": ["sf"], "series": "", "series_index": 0.0, "identifiers": [], "published": "", "rights": "", "rating": 0},
            "added": Utc::now(), "modified": Utc::now(), "size": 1
        }))
        .unwrap();
        for (i, (hid, text, note)) in hs.iter().enumerate() {
            b.highlights.push(Highlight {
                id: hid.to_string(),
                spine: i,
                start: 0,
                end: 3,
                color: "yellow".into(),
                style: String::new(),
                note: note.to_string(),
                text: text.to_string(),
                created: Utc
                    .with_ymd_and_hms(2026, 11, 1 + i as u32, 9, 0, 0)
                    .unwrap(),
            });
        }
        b
    }

    #[test]
    fn rows_carry_book_tags_and_pdf_pages() {
        let mut b = book(
            1,
            "Dune",
            &[
                ("a", "Fear is the mind-killer", "n"),
                ("b", "Pdf quote", ""),
            ],
        );
        b.highlights[1].style = "pdf|background|0,0,1,1".into();
        b.highlights[1].spine = 11;
        let r = rows_of(&[b]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].book_tags, vec!["sf"]);
        assert_eq!(r[0].place(), "Chapter 1");
        assert_eq!(r[1].page, Some(12));
        assert_eq!(r[1].place(), "page 12");
        assert_eq!(r[0].book_author.as_deref(), Some("An Author"));
    }

    #[test]
    fn a_note_is_edited_and_a_highlight_deleted_by_book_and_id() {
        let mut books = vec![book(
            1,
            "Dune",
            &[("a", "x quote", ""), ("b", "y quote", "n")],
        )];
        assert!(set_note(&mut books, 1, "a", "  my thought "));
        assert_eq!(books[0].highlights[0].note, "my thought");
        assert!(!set_note(&mut books, 2, "a", "x"));
        assert!(!set_note(&mut books, 1, "zzz", "x"));
        assert!(delete(&mut books, 1, "b"));
        assert!(!delete(&mut books, 1, "b"), "already gone");
        assert_eq!(books[0].highlights.len(), 1);
    }

    #[test]
    fn the_look_of_a_highlight_changes_but_not_a_pdf_one() {
        let mut b = book(1, "Dune", &[("a", "x quote", ""), ("b", "y quote", "")]);
        b.highlights[1].style = "pdf|background|0,0,1,1".into();
        let mut books = vec![b];
        assert!(set_look(
            &mut books,
            1,
            "a",
            Some("blue"),
            Some("underline")
        ));
        assert_eq!(books[0].highlights[0].color, "blue");
        assert_eq!(books[0].highlights[0].style, "underline");
        assert!(set_look(&mut books, 1, "a", None, Some("squiggly")));
        assert_eq!(books[0].highlights[0].color, "blue");
        assert!(
            !set_look(&mut books, 1, "b", Some("red"), None),
            "PDF rows keep their rectangles"
        );
        assert!(!set_look(&mut books, 1, "zzz", Some("red"), None));
    }

    #[test]
    fn anki_files_come_out_in_each_form_for_highlights_and_vocabulary() {
        let b = book(1, "Dune", &[("a", "Fear is the mind-killer", "n")]);
        let r = rows_of(&[b]);
        let items: Vec<_> = r.iter().map(|x| x.to_item()).collect();
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap();
        let (apkg, n) =
            anki_bytes(AnkiFile::Apkg, CardKind::Basic, "D", &items, None, now).unwrap();
        assert_eq!(n, 1);
        assert_eq!(&apkg[..2], b"PK");
        let (tsv, _) = anki_bytes(
            AnkiFile::Tsv,
            CardKind::Cloze,
            "Books::D",
            &items,
            None,
            now,
        )
        .unwrap();
        let tsv = String::from_utf8(tsv).unwrap();
        assert!(tsv.contains("#notetype:Cloze") && tsv.contains("{{c1::mind-killer}}"));
        let mut store = bookx_lookup::vocab::VocabStore::new();
        store
            .add("ephemeral", Some("An ephemeral joy."), None, now)
            .unwrap();
        let (csv, n) = anki_bytes(
            AnkiFile::Csv,
            CardKind::Basic,
            "V",
            &[],
            Some((&store, &|_| None)),
            now,
        )
        .unwrap();
        assert_eq!(n, 1);
        assert!(String::from_utf8(csv).unwrap().contains("ephemeral"));
        assert_eq!(
            AnkiFile::parse("apkg-cloze"),
            Some((AnkiFile::Apkg, CardKind::Cloze))
        );
        assert_eq!(AnkiFile::parse("pdf-basic"), None);
    }

    #[test]
    fn file_names_follow_the_selection() {
        assert_eq!(file_name("md", 1, Some("Dune")), "Dune - annotations.md");
        assert_eq!(file_name("json", 3, None), "Notes and highlights.json");
    }

    fn with_bookmarks(b: &mut Book) {
        for (i, (id, label)) in [("bm-1", "Page 7"), ("bm-2", "where I stopped")]
            .into_iter()
            .enumerate()
        {
            b.bookmarks.push(crate::core::library::Bookmark {
                id: id.into(),
                spine: i + 1,
                frac: 0.25 * (i + 1) as f64,
                label: label.into(),
                created: Utc.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap(),
            });
        }
    }

    #[test]
    fn bookmarks_are_rows_after_the_highlights_of_their_book() {
        let mut b = book(1, "Dune", &[("a", "Fear is the mind-killer", "n")]);
        with_bookmarks(&mut b);
        let r = rows_of(&[b]);
        assert_eq!(r.len(), 3, "the count of the sidebar is rows().len()");
        assert_eq!(r[1].kind, RowKind::Bookmark);
        assert_eq!(
            (r[1].text.as_str(), r[1].highlight_id.as_str()),
            ("Page 7", "bm-1")
        );
        assert_eq!(r[1].place(), "Chapter 2");
        assert_eq!(r[2].frac, 0.5);
        assert!(r[1].note.is_empty() && r[1].color.is_empty());
    }

    #[test]
    fn a_bookmark_is_deleted_by_id_and_can_be_put_back() {
        let mut b = book(1, "Dune", &[]);
        with_bookmarks(&mut b);
        let mut books = vec![b];
        let removed = delete_bookmark(&mut books, 1, "bm-1").expect("it is there");
        assert_eq!(books[0].bookmarks.len(), 1);
        assert!(delete_bookmark(&mut books, 1, "bm-1").is_none());
        assert!(delete_bookmark(&mut books, 2, "bm-2").is_none());
        let b = &mut books[0];
        assert!(crate::remove_undo::restore(
            &mut b.highlights,
            &mut b.bookmarks,
            removed
        ));
        assert_eq!(b.bookmarks[0].id, "bm-1", "back at its place");
    }

    #[test]
    fn a_bookmark_line_says_where_it_came_from_and_when_the_place_is_rough() {
        let day = Utc.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap();
        assert_eq!(bookmark_line("Page 7", day, None), "Page 7 · 20 Nov 2026");
        let from = ("KOReader (Kobo Libra)".to_string(), false);
        assert_eq!(
            bookmark_line("Page 7", day, Some(&from)),
            "Page 7 · 20 Nov 2026 · KOReader (Kobo Libra)"
        );
        let rough = ("Kobo".to_string(), true);
        assert!(bookmark_line("Page 7", day, Some(&rough)).ends_with("Kobo · approximate place"));
    }

    #[test]
    fn bookmarks_go_into_an_export_after_the_highlights() {
        let mut b = book(1, "Dune", &[("a", "Fear is the mind-killer", "n")]);
        with_bookmarks(&mut b);
        let rows = rows_of(&[b]);
        let items: Vec<_> = rows.iter().map(|r| r.to_item()).collect();
        let md = crate::annotation_export::render(&items, "md", Utc::now()).unwrap();
        let (h, m) = (md.find("mind-killer").unwrap(), md.find("Page 7").unwrap());
        assert!(h < m, "highlights first:\n{md}");
        assert!(md.contains("where I stopped"));
    }
}
