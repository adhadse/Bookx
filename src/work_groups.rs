//! Groups of editions in the library grid (roadmap PP-G-5): books that are the same work are shown
//! as one card with a badge and an edition list. The grouping only decides what is shown; no file
//! and no catalogue entry is merged or deleted here (merging is Find Duplicates, with its own
//! confirmation). Three sources, strongest first:
//!
//! 1. links the user made (`Link as editions`), stored in the catalogue;
//! 2. the same ISBN, or the same work identifier (`work`, `workid`, `goodreads_work`, `olw`, ...);
//! 3. the same normalised title (subtitles and articles forgiven) by the same author.
//!
//! A translation is never grouped by the third rule (the languages differ), a book the user took
//! out (`Unlink`) is never pulled back in by rules 2 and 3, and a book with another subtitle,
//! another number ("Part 2") or another author stays alone.
//!
//! No GTK in here, so it is unit tested; the view is `ui/library.rs` and `ui/library_works.rs`.

use std::collections::{HashMap, HashSet};

use bookx_sources::authors::to_first_last;
use bookx_sources::dedup::title_key;
use bookx_sources::isbn;
use bookx_sources::text::normalize;

use crate::core::library::Book;

/// What the user decided, loaded from the catalogue (`SqliteStore::work_links` / `work_apart`).
#[derive(Debug, Clone, Default)]
pub struct Links {
    /// Book id → work id.
    pub work_of: HashMap<u64, u64>,
    /// The edition that opens first (book ids, at most one per work).
    pub preferred: HashSet<u64>,
    /// Books taken out of the automatic grouping.
    pub apart: HashSet<u64>,
}

impl Links {
    pub fn from_store(store: &bookx_store::SqliteStore) -> Links {
        let mut l = Links::default();
        for k in store.work_links() {
            l.work_of.insert(k.book, k.work);
            if k.preferred {
                l.preferred.insert(k.book);
            }
        }
        l.apart = store.work_apart().into_iter().collect();
        l
    }
}

/// One card of the grid: a single book, or several editions of one work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// The edition that stands for the card and opens first.
    pub rep: u64,
    /// All books of the card in display order, `rep` first.
    pub members: Vec<u64>,
    /// At least two of them were linked by the user.
    pub linked: bool,
    /// The edition whose reading progress the card shows.
    pub progress_from: u64,
}

impl Card {
    pub fn is_group(&self) -> bool {
        self.members.len() > 1
    }
}

/// How the formats rank when no edition was chosen and none was read: KEPUB and EPUB first.
fn format_rank(file: &str) -> usize {
    const ORDER: [&str; 8] = ["kepub", "epub", "azw3", "mobi", "fb2", "pdf", "cbz", "txt"];
    let f = crate::works::format_of(file);
    ORDER.iter().position(|x| *x == f).unwrap_or(ORDER.len())
}

/// The first two letters of the primary language subtag: `en-US` → `en`, `eng` → `en`.
fn lang_key(l: &str) -> String {
    l.trim()
        .to_lowercase()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .chars()
        .take(2)
        .collect()
}

/// Two books may be editions when their languages agree. A book without a language agrees with
/// everything only while the bucket has at most one language; with two (English and German) it
/// could be either, so it only goes with books that have no language either.
fn languages_compatible(a: &str, b: &str, bucket_langs: usize) -> bool {
    let (a, b) = (lang_key(a), lang_key(b));
    match (a.is_empty(), b.is_empty()) {
        (false, false) => a == b,
        (true, true) => true,
        _ => bucket_langs <= 1,
    }
}

/// `(surname, first initial)` of every author, normalised.
fn author_keys(b: &Book) -> Vec<(String, String)> {
    b.meta
        .authors
        .iter()
        .filter_map(|a| {
            let n = normalize(&to_first_last(a));
            let mut words = n.split_whitespace();
            let first = words.next()?.to_string();
            let sur = n.split_whitespace().last()?.to_string();
            let initial: String = if n.split_whitespace().count() > 1 {
                first.chars().take(1).collect()
            } else {
                String::new()
            };
            Some((sur, initial))
        })
        .collect()
}

fn authors_match(a: &[(String, String)], b: &[(String, String)]) -> bool {
    a.iter().any(|x| {
        b.iter()
            .any(|y| x.0 == y.0 && (x.1.is_empty() || y.1.is_empty() || x.1 == y.1))
    })
}

const WORK_SCHEMES: [&str; 7] = [
    "work",
    "workid",
    "work_id",
    "goodreads_work",
    "olw",
    "openlibrary_work",
    "olworkid",
];

/// The ISBN-13s and work identifiers of a book, as bucket keys.
fn id_keys(b: &Book) -> Vec<String> {
    let mut out = Vec::new();
    for i in &b.meta.identifiers {
        let scheme = i.scheme.trim().to_lowercase();
        let value = i.value.trim();
        if value.is_empty() {
            continue;
        }
        if scheme == "isbn" {
            if let Some(c) = isbn::canonical(value) {
                out.push(format!("isbn:{c}"));
            }
        } else if WORK_SCHEMES.contains(&scheme.as_str()) {
            out.push(format!("work:{scheme}:{}", value.to_lowercase()));
        }
    }
    out
}

struct Prep {
    title: String,
    sub: String,
    authors: Vec<(String, String)>,
}

struct Dsu(Vec<usize>);

impl Dsu {
    fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] != x {
            self.0[x] = self.0[self.0[x]];
            x = self.0[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            // the earlier book stays the root, so cards keep their place in the list
            let (lo, hi) = (a.min(b), a.max(b));
            self.0[hi] = lo;
        }
    }
}

/// Cards for the books in display order. A book that is alone makes a card of one. The cards come
/// in the order of their first book.
pub fn group(books: &[&Book], links: &Links) -> Vec<Card> {
    let n = books.len();
    let mut dsu = Dsu((0..n).collect());
    let mut linked_roots: HashSet<usize> = HashSet::new();

    // 1. what the user linked
    let mut by_work: HashMap<u64, usize> = HashMap::new();
    for (i, b) in books.iter().enumerate() {
        if let Some(w) = links.work_of.get(&b.id) {
            match by_work.get(w) {
                Some(&j) => {
                    dsu.union(i, j);
                    linked_roots.insert(i);
                    linked_roots.insert(j);
                }
                None => {
                    by_work.insert(*w, i);
                }
            }
        }
    }

    // 2. identifiers
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for (i, b) in books.iter().enumerate() {
        if links.apart.contains(&b.id) {
            continue;
        }
        for k in id_keys(b) {
            match by_key.get(&k) {
                Some(&j) => dsu.union(i, j),
                None => {
                    by_key.insert(k, i);
                }
            }
        }
    }

    // 3. title and author, within buckets of the same main title
    let mut buckets: HashMap<String, Vec<(usize, Prep)>> = HashMap::new();
    for (i, b) in books.iter().enumerate() {
        if links.apart.contains(&b.id) {
            continue;
        }
        let k = title_key(&b.meta.title);
        if k.main.is_empty() {
            continue;
        }
        let authors = author_keys(b);
        if authors.is_empty() {
            continue;
        }
        buckets.entry(k.main.clone()).or_default().push((
            i,
            Prep {
                title: k.main,
                sub: k.sub,
                authors,
            },
        ));
    }
    for list in buckets.values() {
        let langs: HashSet<String> = list
            .iter()
            .map(|(i, _)| lang_key(&books[*i].meta.language))
            .filter(|l| !l.is_empty())
            .collect();
        for (x, (i, a)) in list.iter().enumerate() {
            for (j, b) in &list[x + 1..] {
                debug_assert_eq!(a.title, b.title);
                if !(a.sub.is_empty() || b.sub.is_empty() || a.sub == b.sub) {
                    continue;
                }
                if !languages_compatible(
                    &books[*i].meta.language,
                    &books[*j].meta.language,
                    langs.len(),
                ) {
                    continue;
                }
                if authors_match(&a.authors, &b.authors) {
                    dsu.union(*i, *j);
                }
            }
        }
    }

    // collect
    let mut order: Vec<usize> = Vec::new();
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = dsu.find(i);
        let e = members.entry(r).or_default();
        if e.is_empty() {
            order.push(r);
        }
        e.push(i);
    }
    let linked: HashSet<usize> = linked_roots.into_iter().map(|i| dsu.find(i)).collect();
    order
        .into_iter()
        .map(|r| {
            let idx = &members[&r];
            let rep = pick_preferred(books, idx, links);
            let progress_from = pick_progress(books, idx, rep);
            let mut ids: Vec<u64> = vec![books[rep].id];
            ids.extend(idx.iter().filter(|i| **i != rep).map(|i| books[*i].id));
            Card {
                rep: books[rep].id,
                members: ids,
                linked: linked.contains(&r),
                progress_from: books[progress_from].id,
            }
        })
        .collect()
}

/// The chosen edition, else the one read last, else the best format, else the first.
fn pick_preferred(books: &[&Book], idx: &[usize], links: &Links) -> usize {
    if idx.len() == 1 {
        return idx[0];
    }
    if let Some(i) = idx
        .iter()
        .find(|i| links.preferred.contains(&books[**i].id))
    {
        return *i;
    }
    *idx.iter()
        .min_by_key(|i| {
            let b = books[**i];
            (
                std::cmp::Reverse(b.last_read.map(|t| t.timestamp())),
                format_rank(&b.file),
                **i,
            )
        })
        .unwrap_or(&idx[0])
}

/// Progress comes from the edition read last; none read: the preferred one.
fn pick_progress(books: &[&Book], idx: &[usize], rep: usize) -> usize {
    idx.iter()
        .filter(|i| books[**i].last_read.is_some() && books[**i].progress > 0.0)
        .max_by_key(|i| books[**i].last_read.map(|t| t.timestamp()))
        .copied()
        .unwrap_or(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::opf::Identifier;
    use chrono::{TimeZone, Utc};

    fn book(id: u64, file: &str, title: &str, authors: &[&str]) -> Book {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":["A"],"author_sort":"A","publisher":"","language":"","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        b.id = id;
        b.file = file.into();
        b.meta.title = title.into();
        b.meta.authors = authors.iter().map(|s| s.to_string()).collect();
        b
    }

    fn ident(b: &mut Book, scheme: &str, value: &str) {
        b.meta.identifiers.push(Identifier {
            scheme: scheme.into(),
            value: value.into(),
            xml_id: None,
        });
    }

    fn cards(books: &[Book], links: &Links) -> Vec<Vec<u64>> {
        let refs: Vec<&Book> = books.iter().collect();
        group(&refs, links).into_iter().map(|c| c.members).collect()
    }

    #[test]
    fn same_title_and_author_form_one_card_and_keep_the_order() {
        let books = vec![
            book(1, "Dune.epub", "Dune", &["Frank Herbert"]),
            book(2, "Emma.epub", "Emma", &["Jane Austen"]),
            book(3, "Dune.azw3", "The Dune", &["Herbert, Frank"]),
            book(
                4,
                "Dune.kepub.epub",
                "Dune: Deluxe Edition",
                &["F. Herbert"],
            ),
        ];
        let c = cards(&books, &Links::default());
        assert_eq!(
            c,
            vec![vec![4, 1, 3], vec![2]],
            "the KEPUB leads, then the others in order"
        );
    }

    #[test]
    fn unicode_titles_group_across_accents_and_scripts() {
        let books = vec![
            book(1, "a.epub", "Les Misérables", &["Victor Hugo"]),
            book(2, "b.epub", "LES MISERABLES", &["Hugo, Victor"]),
            book(3, "c.epub", "Война и мир", &["Лев Толстой"]),
            book(4, "d.epub", "война и мир", &["Толстой, Лев"]),
            book(5, "e.epub", "吾輩は猫である", &["夏目漱石"]),
            book(6, "f.epub", "吾輩は猫である", &["夏目漱石"]),
        ];
        assert_eq!(
            cards(&books, &Links::default()),
            vec![vec![1, 2], vec![3, 4], vec![5, 6]]
        );
    }

    #[test]
    fn subtitles_numbers_and_authors_keep_books_apart() {
        let books = vec![
            book(1, "a.epub", "Python: The Basics", &["Ann Lee"]),
            book(2, "b.epub", "Python: Advanced Topics", &["Ann Lee"]),
            book(3, "c.epub", "Foundation 1", &["Isaac Asimov"]),
            book(4, "d.epub", "Foundation 2", &["Isaac Asimov"]),
            book(5, "e.epub", "Dune", &["Frank Herbert"]),
            book(6, "f.epub", "Dune", &["Brian Herbert"]),
            book(7, "g.epub", "Dune", &["Someone Else"]),
        ];
        let c = cards(&books, &Links::default());
        assert_eq!(c.len(), 7, "{c:?}");
    }

    #[test]
    fn a_translation_is_not_an_edition() {
        let mut en = book(1, "a.epub", "Hamlet", &["William Shakespeare"]);
        en.meta.language = "en".into();
        let mut en_us = book(2, "b.epub", "Hamlet", &["William Shakespeare"]);
        en_us.meta.language = "en-US".into();
        let mut de = book(3, "c.epub", "Hamlet", &["William Shakespeare"]);
        de.meta.language = "de".into();
        let none = book(4, "d.epub", "Hamlet", &["William Shakespeare"]);
        let c = cards(
            &[en.clone(), en_us.clone(), de, none.clone()],
            &Links::default(),
        );
        assert_eq!(
            c,
            vec![vec![1, 2], vec![3], vec![4]],
            "no language: could be either"
        );
        // with one language in sight a book without language goes with it
        let c = cards(&[en, en_us, none], &Links::default());
        assert_eq!(c, vec![vec![1, 2, 4]]);
    }

    #[test]
    fn a_title_alone_never_groups_books_without_authors() {
        let books = vec![
            book(1, "a.epub", "Introduction", &[]),
            book(2, "b.epub", "Introduction", &[]),
        ];
        assert_eq!(cards(&books, &Links::default()).len(), 2);
    }

    #[test]
    fn several_authors_match_on_any_shared_author() {
        let books = vec![
            book(
                1,
                "a.epub",
                "Good Omens",
                &["Terry Pratchett", "Neil Gaiman"],
            ),
            book(2, "b.epub", "Good Omens", &["Neil Gaiman"]),
            book(3, "c.epub", "Good Omens", &["Terry Pratchett"]),
            book(4, "d.epub", "Good Omens", &["Stephen King"]),
        ];
        let c = cards(&books, &Links::default());
        assert_eq!(c, vec![vec![1, 2, 3], vec![4]]);
    }

    #[test]
    fn the_same_isbn_groups_even_when_the_titles_differ() {
        let mut a = book(1, "a.epub", "Der Name der Rose", &["Umberto Eco"]);
        let mut b = book(2, "b.epub", "Il nome della rosa", &["U. Eco"]);
        ident(&mut a, "isbn", "978-0-306-40615-7");
        ident(&mut b, "ISBN", "0306406152");
        let mut c = book(3, "c.epub", "Other", &["Someone"]);
        ident(&mut c, "isbn", "9780306406157");
        let d = book(4, "d.epub", "Another", &["Someone"]);
        assert_eq!(
            cards(&[a, b, c, d], &Links::default()),
            vec![vec![1, 2, 3], vec![4]]
        );
    }

    #[test]
    fn a_work_identifier_groups() {
        let mut a = book(1, "a.epub", "One", &["A B"]);
        let mut b = book(2, "b.epub", "Two", &["C D"]);
        ident(&mut a, "goodreads_work", "123");
        ident(&mut b, "goodreads_work", "123");
        assert_eq!(cards(&[a, b], &Links::default()), vec![vec![1, 2]]);
    }

    #[test]
    fn user_links_group_unrelated_titles_and_unlinked_books_stay_out() {
        let books = [
            book(1, "a.epub", "Dune", &["Frank Herbert"]),
            book(2, "b.epub", "Dune", &["Frank Herbert"]),
            book(3, "c.epub", "Duna", &["Frank Herbert"]),
            book(4, "d.epub", "Dune", &["Frank Herbert"]),
        ];
        let mut l = Links::default();
        l.work_of.insert(1, 1);
        l.work_of.insert(3, 1);
        l.apart.insert(4);
        let c = group(&books.iter().collect::<Vec<_>>(), &l);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].members, vec![1, 2, 3]);
        assert!(c[0].linked);
        assert_eq!(c[1].members, vec![4]);
    }

    #[test]
    fn a_link_wins_over_apart_for_the_linked_books_only() {
        // `apart` keeps a book out of the automatic rules; an explicit link still counts
        let books = vec![
            book(1, "a.epub", "Dune", &["Frank Herbert"]),
            book(2, "b.epub", "Dune", &["Frank Herbert"]),
        ];
        let mut l = Links::default();
        l.apart.insert(2);
        assert_eq!(cards(&books, &l).len(), 2);
        l.work_of.insert(1, 5);
        l.work_of.insert(2, 5);
        assert_eq!(cards(&books, &l), vec![vec![1, 2]]);
    }

    #[test]
    fn the_chosen_edition_stands_for_the_card() {
        let books = [
            book(1, "a.epub", "Dune", &["Frank Herbert"]),
            book(2, "b.azw3", "Dune", &["Frank Herbert"]),
        ];
        let mut l = Links::default();
        l.preferred.insert(2);
        let c = group(&books.iter().collect::<Vec<_>>(), &l);
        assert_eq!(c[0].rep, 2);
        assert_eq!(c[0].members, vec![2, 1]);
    }

    #[test]
    fn without_a_choice_the_last_read_then_the_best_format_leads() {
        let mut books = [
            book(1, "a.pdf", "Dune", &["Frank Herbert"]),
            book(2, "b.azw3", "Dune", &["Frank Herbert"]),
            book(3, "c.epub", "Dune", &["Frank Herbert"]),
        ];
        let c = group(&books.iter().collect::<Vec<_>>(), &Links::default());
        assert_eq!(c[0].rep, 3, "epub beats azw3 and pdf");
        books[1].last_read = Some(Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap());
        books[1].progress = 0.4;
        let c = group(&books.iter().collect::<Vec<_>>(), &Links::default());
        assert_eq!(c[0].rep, 2, "the one read last leads");
        assert_eq!(c[0].progress_from, 2);
    }

    #[test]
    fn progress_follows_the_edition_read_last_even_when_another_is_preferred() {
        let mut books = [
            book(1, "a.epub", "Dune", &["Frank Herbert"]),
            book(2, "b.epub", "Dune", &["Frank Herbert"]),
        ];
        books[0].progress = 0.9;
        books[0].last_read = Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
        books[1].progress = 0.2;
        books[1].last_read = Some(Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap());
        let mut l = Links::default();
        l.preferred.insert(1);
        let c = group(&books.iter().collect::<Vec<_>>(), &l);
        assert_eq!(c[0].rep, 1);
        assert_eq!(c[0].progress_from, 2);
    }

    #[test]
    fn empty_input_gives_no_cards() {
        assert!(group(&[], &Links::default()).is_empty());
    }

    /// Budget (release): grouping 20 000 books, a fifth of them editions of another, in 400 ms.
    /// Debug builds get 30x, `BOOKX_PERF_SCALE` scales it (like `crates/*/tests/perf_budgets.rs`).
    #[test]
    fn grouping_twenty_thousand_books_stays_within_the_budget() {
        let books: Vec<Book> = (0..20_000u64)
            .map(|i| {
                let n = if i % 5 == 4 { i - 1 } else { i };
                let mut b = book(
                    i + 1,
                    if i % 2 == 0 { "a.epub" } else { "a.azw3" },
                    &format!("A Long Book Title Number {n}: Subtitle"),
                    &[&format!("Firstname{} Surname{}", n % 977, n % 3001)],
                );
                b.meta.language = "en".into();
                b
            })
            .collect();
        let refs: Vec<&Book> = books.iter().collect();
        let t = std::time::Instant::now();
        let cards = group(&refs, &Links::default());
        let took = t.elapsed();
        assert_eq!(cards.len(), 16_000);
        let debug = if cfg!(debug_assertions) { 30.0 } else { 1.0 };
        let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        let budget = std::time::Duration::from_secs_f64(0.4 * debug * scale);
        assert!(took <= budget, "grouping took {took:?}, budget {budget:?}");
    }
}
