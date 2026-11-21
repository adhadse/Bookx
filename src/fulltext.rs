//! Full-text search across the contents of all books (roadmap L2/L33), on top of the SQLite
//! full-text index of `bookx-store`. The searchable text of every book lives in the library's own
//! database (`bookx-library.db`), so it travels with the library folder.
//!
//! In the search box, `text:word` (or `text:"exact words"`) finds books whose *contents* match;
//! it combines with the normal fields, for example `author:tolkien text:"one ring"`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use bookx_store::{SqliteStore, TextHit};

use crate::core::book::OpenBook;
use crate::core::library::Book;

/// Splits the `text:` terms off a query: `(rest of the query, phrases to find in the contents)`.
pub fn split_text_terms(query: &str) -> (String, Vec<String>) {
    let mut rest = Vec::new();
    let mut terms = Vec::new();
    let chars: Vec<char> = query.chars().collect();
    let mut i = 0;
    let mut cur = String::new();
    let flush = |cur: &mut String, rest: &mut Vec<String>| {
        if !cur.is_empty() {
            rest.push(std::mem::take(cur));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            flush(&mut cur, &mut rest);
            i += 1;
            continue;
        }
        // a token starts here
        let negated = c == '-';
        let start = if negated { i + 1 } else { i };
        let is_text = !negated
            && chars[start..]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase()
                .starts_with("text:");
        if is_text {
            let mut j = start + 5;
            let mut value = String::new();
            if j < chars.len() && chars[j] == '"' {
                j += 1;
                while j < chars.len() && chars[j] != '"' {
                    value.push(chars[j]);
                    j += 1;
                }
                j += 1; // closing quote (or end)
            } else {
                while j < chars.len() && !chars[j].is_whitespace() {
                    value.push(chars[j]);
                    j += 1;
                }
            }
            let value = value.trim().to_string();
            if !value.is_empty() {
                terms.push(value);
            }
            i = j;
            continue;
        }
        // ordinary token, keeping quotes together
        let mut in_quote = false;
        while i < chars.len() && (in_quote || !chars[i].is_whitespace()) {
            if chars[i] == '"' {
                in_quote = !in_quote;
            }
            cur.push(chars[i]);
            i += 1;
        }
        flush(&mut cur, &mut rest);
    }
    flush(&mut cur, &mut rest);
    (rest.join(" "), terms)
}

/// Plain text of a whole book, chapters separated by blank lines.
pub fn extract_text(epub: &Path) -> Result<String> {
    let book = OpenBook::open(epub).with_context(|| format!("opening {}", epub.display()))?;
    let mut out = String::new();
    for p in book.spine_paths() {
        if let Some(doc) = book.chapter_doc(&p) {
            let t = doc.to_text();
            if !t.trim().is_empty() {
                out.push_str(&t);
                out.push_str("\n\n");
            }
        }
    }
    Ok(out)
}

/// What changes when a book's file changes.
fn stamp(b: &Book) -> i64 {
    b.modified.timestamp_millis()
}

/// Handle to the index; cheap to clone and safe to use from worker threads.
#[derive(Clone)]
pub struct FullText {
    store: Arc<SqliteStore>,
    /// An indexing run is in progress (only one at a time).
    busy: Arc<AtomicBool>,
}

impl FullText {
    /// The index of the library whose database is `store`.
    pub fn new(store: Arc<SqliteStore>) -> FullText {
        FullText {
            store,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Ids of the books whose text is missing or out of date.
    pub fn pending(&self, books: &[Book]) -> Vec<u64> {
        let stamps = self.store.text_stamps();
        books
            .iter()
            .filter(|b| stamps.get(&b.id) != Some(&stamp(b)))
            .map(|b| b.id)
            .collect()
    }

    /// Stores the text of one book. The book must be in the library database.
    pub fn index_book(&self, book: &Book, text: &str) -> Result<()> {
        self.store
            .index_book_text_stamped(book.id, stamp(book), text)
    }

    /// Indexes every book that is missing or out of date. Books that cannot be read are skipped.
    /// Returns how many were indexed, or `None` when another run is already going. Blocking:
    /// call it from a worker thread.
    pub fn index_pending(&self, root: &Path, books: &[Book]) -> Option<usize> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return None;
        }
        let mut done = 0;
        let pending = self.pending(books);
        for b in books.iter().filter(|b| pending.contains(&b.id)) {
            let Ok(text) = extract_text(&root.join(&b.file)) else {
                continue;
            };
            if self.index_book(b, &text).is_ok() {
                done += 1;
            }
        }
        self.busy.store(false, Ordering::SeqCst);
        Some(done)
    }

    /// Forgets every indexed text; the caller starts indexing again (`Shared::start_indexing`).
    /// Returns how many books had text indexed.
    pub fn clear(&self) -> Result<usize> {
        self.store.clear_text_index()
    }

    /// Whether an indexing run is going right now.
    pub fn is_running(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn indexed_books(&self) -> usize {
        self.store.indexed_text_count()
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<TextHit>> {
        self.store.search_text(query, limit)
    }

    /// Ids of the books matching *all* phrases, best match of the first phrase first.
    pub fn matching_books(&self, phrases: &[String], limit: usize) -> Result<Vec<u64>> {
        let mut result: Option<Vec<u64>> = None;
        for p in phrases {
            let ids: Vec<u64> = self.search(p, limit)?.into_iter().map(|h| h.id).collect();
            result = Some(match result {
                None => ids,
                Some(prev) => prev.into_iter().filter(|i| ids.contains(i)).collect(),
            });
        }
        Ok(result.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-ft-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn book(id: u64, ts: i64) -> Book {
        use chrono::TimeZone;
        let when = chrono::Utc.timestamp_millis_opt(ts).unwrap();
        Book {
            id,
            file: format!("{id}.epub"),
            cover: None,
            meta: crate::core::opf::Metadata {
                title: format!("Book {id}"),
                ..Default::default()
            },
            added: when,
            modified: when,
            last_read: None,
            size: 1,
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

    /// An index over an in-memory library database that already holds `books`.
    fn index_of(books: &[Book]) -> (FullText, Arc<SqliteStore>) {
        use bookx_core::contracts::Store;
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        for b in books {
            store.put(b.clone()).unwrap();
        }
        (FullText::new(store.clone()), store)
    }

    #[test]
    fn splitting_text_terms_off_a_query() {
        let s = |q: &str| split_text_terms(q);
        assert_eq!(s(""), (String::new(), vec![]));
        assert_eq!(s("tolkien"), ("tolkien".into(), vec![]));
        assert_eq!(s("text:ring"), (String::new(), vec!["ring".to_string()]));
        assert_eq!(
            s("author:tolkien text:ring"),
            ("author:tolkien".into(), vec!["ring".to_string()])
        );
        assert_eq!(
            s(r#"text:"one ring" tag:fantasy"#),
            ("tag:fantasy".into(), vec!["one ring".to_string()])
        );
        assert_eq!(
            s(r#"TEXT:Mordor -tag:x text:"dark lord""#),
            (
                "-tag:x".into(),
                vec!["Mordor".to_string(), "dark lord".to_string()]
            )
        );
        assert_eq!(s("text: a"), ("a".into(), vec![]));
        assert_eq!(
            s(r#"text:"open end"#),
            (String::new(), vec!["open end".to_string()])
        );
        assert_eq!(
            s(r#"author:"le guin" text:wind"#),
            (r#"author:"le guin""#.into(), vec!["wind".to_string()])
        );
        assert_eq!(s("-text:x"), ("-text:x".into(), vec![]));
    }

    #[test]
    fn stamps_decide_what_is_pending() {
        let books = [book(1, 10), book(2, 20)];
        let (ft, _) = index_of(&books);
        assert_eq!(ft.pending(&books), vec![1, 2]);
        ft.index_book(&books[0], "some words here").unwrap();
        assert_eq!(ft.pending(&books), vec![2]);
        // the book changed: pending again
        assert_eq!(ft.pending(&[book(1, 11)]), vec![1]);
        assert_eq!(ft.indexed_books(), 1);
    }

    #[test]
    fn the_index_lives_in_the_database_and_survives_a_restart() {
        let d = tmp("restart");
        let db = d.join("lib.db");
        let b = book(5, 99);
        {
            use bookx_core::contracts::Store;
            let store = Arc::new(SqliteStore::open(&db).unwrap());
            store.put(b.clone()).unwrap();
            FullText::new(store)
                .index_book(&b, "persistent text")
                .unwrap();
        }
        let ft = FullText::new(Arc::new(SqliteStore::open(&db).unwrap()));
        assert!(ft.pending(std::slice::from_ref(&b)).is_empty());
        assert_eq!(ft.search("persistent", 5).unwrap().len(), 1);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn searching_finds_phrases_and_requires_all_words() {
        let books = [book(1, 1), book(2, 1), book(3, 1)];
        let (ft, _) = index_of(&books);
        ft.index_book(&books[0], "The quick brown fox jumps over the lazy dog")
            .unwrap();
        ft.index_book(&books[1], "A slow brown bear sleeps all winter")
            .unwrap();
        ft.index_book(&books[2], "Nothing relevant here").unwrap();
        let ids = |q: &str| -> Vec<u64> {
            let mut v: Vec<u64> = ft
                .search(q, 10)
                .unwrap()
                .into_iter()
                .map(|h| h.id)
                .collect();
            v.sort();
            v
        };
        assert_eq!(ids("brown"), vec![1, 2]);
        assert_eq!(ids("brown fox"), vec![1]);
        assert_eq!(ids("sleeping"), vec![2], "stemmed matching");
        assert!(ids("absent").is_empty());
        let hit = &ft.search("fox", 5).unwrap()[0];
        assert!(hit.snippet.contains("<mark>fox</mark>"), "{}", hit.snippet);
        assert_eq!(
            ft.matching_books(&["brown".into(), "lazy".into()], 10)
                .unwrap(),
            vec![1]
        );
        assert!(ft.matching_books(&[], 10).unwrap().is_empty());
    }

    #[test]
    fn hostile_queries_do_not_break_the_search() {
        let books = [book(1, 1)];
        let (ft, _) = index_of(&books);
        ft.index_book(&books[0], "plain text").unwrap();
        for q in [
            "\"",
            "AND OR NOT",
            "a*",
            "(",
            "col:umn",
            "'; DROP TABLE x; --",
            "",
        ] {
            let _ = ft.search(q, 5);
        }
        assert_eq!(ft.search("plain", 5).unwrap().len(), 1);
    }

    #[test]
    fn extracts_the_text_of_every_chapter() {
        let d = tmp("extract");
        let md = crate::core::opf::Metadata {
            title: "T".into(),
            authors: vec!["A".into()],
            ..Default::default()
        };
        let ch = |t: &str, b: &str| crate::core::convert::Chapter {
            title: t.into(),
            body: b.into(),
        };
        let p = d.join("b.epub");
        fs::write(
            &p,
            crate::core::convert::create_epub(
                &md,
                &[
                    ch("One", "<p>alpha words</p>"),
                    ch("Two", "<p>beta words</p>"),
                ],
                None,
            )
            .to_bytes()
            .unwrap(),
        )
        .unwrap();
        let text = extract_text(&p).unwrap();
        assert!(text.contains("alpha words") && text.contains("beta words"));
        assert!(extract_text(&d.join("missing.epub")).is_err());
        let _ = fs::remove_dir_all(d);
    }

    fn real_book(dir: &Path, id: u64, text: &str) -> Book {
        let md = crate::core::opf::Metadata {
            title: format!("Real {id}"),
            authors: vec!["A".into()],
            ..Default::default()
        };
        let ch = crate::core::convert::Chapter {
            title: "One".into(),
            body: format!("<p>{text}</p>"),
        };
        let file = format!("{id}.epub");
        fs::write(
            dir.join(&file),
            crate::core::convert::create_epub(&md, &[ch], None)
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        let mut b = book(id, 1000 + id as i64);
        b.file = file;
        b
    }

    #[test]
    fn indexing_pending_books_reads_the_files_once() {
        let d = tmp("pending");
        let books = vec![
            real_book(&d, 1, "narwhals swim north"),
            real_book(&d, 2, "elephants walk south"),
        ];
        let (ft, store) = index_of(&books);
        assert_eq!(ft.index_pending(&d, &books), Some(2));
        assert_eq!(ft.matching_books(&["narwhals".into()], 5).unwrap(), vec![1]);
        // nothing changed: nothing to do
        assert_eq!(ft.index_pending(&d, &books), Some(0));
        // a changed book is indexed again, the other is left alone
        let mut changed = books.clone();
        changed[1] = real_book(&d, 2, "giraffes walk east");
        changed[1].modified += chrono::Duration::seconds(5);
        {
            use bookx_core::contracts::Store;
            store.put(changed[1].clone()).unwrap();
        }
        assert_eq!(ft.index_pending(&d, &changed), Some(1));
        assert_eq!(ft.matching_books(&["giraffes".into()], 5).unwrap(), vec![2]);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn unreadable_books_are_skipped_not_fatal() {
        let d = tmp("skip");
        let mut broken = book(9, 5);
        broken.file = "missing.epub".into();
        let good = real_book(&d, 1, "readable words");
        let books = [broken, good];
        let (ft, _) = index_of(&books);
        assert_eq!(ft.index_pending(&d, &books), Some(1));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_book_that_left_the_library_is_skipped() {
        let d = tmp("gone");
        let in_db = real_book(&d, 1, "still here");
        let gone = real_book(&d, 2, "already removed");
        let (ft, _) = index_of(std::slice::from_ref(&in_db));
        // book 2 is in the snapshot the indexer got, but no longer in the database
        assert_eq!(ft.index_pending(&d, &[in_db, gone]), Some(1));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn only_one_indexing_run_at_a_time() {
        let d = tmp("busy");
        let b = real_book(&d, 1, "x words");
        let (ft, _) = index_of(std::slice::from_ref(&b));
        ft.busy.store(true, Ordering::SeqCst);
        assert_eq!(ft.index_pending(&d, std::slice::from_ref(&b)), None);
        ft.busy.store(false, Ordering::SeqCst);
        assert_eq!(ft.index_pending(&d, std::slice::from_ref(&b)), Some(1));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn indexing_from_many_threads_is_safe() {
        let books: Vec<Book> = (0..80).map(|i| book(i, 1)).collect();
        let (ft, _) = index_of(&books);
        std::thread::scope(|s| {
            for t in 0..8u64 {
                let ft = ft.clone();
                let books = &books;
                s.spawn(move || {
                    for i in 0..10u64 {
                        let id = t * 10 + i;
                        ft.index_book(
                            &books[id as usize],
                            &format!("book number {id} mentions zebra{t}"),
                        )
                        .unwrap();
                    }
                });
            }
        });
        assert_eq!(ft.indexed_books(), 80);
        assert_eq!(ft.search("zebra3", 100).unwrap().len(), 10);
    }
}
