mod common;

use bookx_core::contracts::Store;
use bookx_core::library::Book;
use bookx_core::opf::Identifier;
use bookx_core::query::Query;
use bookx_store::{is_fast_query, SqliteStore};
use common::*;

fn store_with(books: &[Book]) -> SqliteStore {
    let s = SqliteStore::open_in_memory().unwrap();
    s.put_many(books).unwrap();
    s
}

#[test]
fn fast_query_detection() {
    assert!(is_fast_query("dune"));
    assert!(is_fast_query("  Dune2 "));
    assert!(!is_fast_query("du"), "too short for trigrams");
    assert!(!is_fast_query("two words"));
    assert!(!is_fast_query("-dune"));
    assert!(!is_fast_query("tag:fantasy"));
    assert!(!is_fast_query("\"dune\""));
    assert!(!is_fast_query("ünï"));
    assert!(!is_fast_query(""));
}

#[test]
fn metadata_fields_are_all_searchable() {
    let mut b = book(1, "Zebra Crossing", "Quentin Author");
    b.meta.series = "Savannah Saga".into();
    b.meta.tags = vec!["wildlife".into(), "travel".into()];
    b.meta.publisher = "Pinnacle House".into();
    b.meta.description = "an unrelated blurb about giraffes".into();
    let s = store_with(&[b]);
    for q in [
        "zebra", "quentin", "savannah", "wildlife", "travel", "pinnacle",
    ] {
        assert_eq!(s.search_ids(q), vec![1], "{q}");
    }
    // description is only found by the scoped query, like the pure query engine
    assert!(s.search_ids("giraffes").is_empty());
    assert_eq!(s.search_ids("comments:giraffes"), vec![1]);
    assert!(s.search_ids("nonexistent").is_empty());
}

#[test]
fn index_follows_update_and_remove() {
    let s = store_with(&[book(1, "Alpha Tale", "One"), book(2, "Beta Tale", "Two")]);
    assert_eq!(s.search_ids("tale"), vec![1, 2]);
    let mut changed = book(1, "Gamma Story", "One");
    changed.meta.tags = vec!["newtag".into()];
    s.put(changed).unwrap();
    assert_eq!(s.search_ids("tale"), vec![2]);
    assert!(s.search_ids("alpha").is_empty());
    assert_eq!(s.search_ids("gamma"), vec![1]);
    assert_eq!(s.search_ids("newtag"), vec![1]);
    s.remove(2).unwrap();
    assert!(s.search_ids("tale").is_empty());
    assert!(s.check_integrity().is_empty());
}

#[test]
fn text_search_ranks_and_highlights() {
    let s = store_with(&[book(1, "a", "x"), book(2, "b", "x"), book(3, "c", "x")]);
    s.index_book_text(1, "The quick brown fox jumps over the lazy dog.")
        .unwrap();
    s.index_book_text(2, "fox fox fox fox fox, a story about a fox")
        .unwrap();
    s.index_book_text(3, "Nothing relevant lives here.")
        .unwrap();
    assert_eq!(s.indexed_text_count(), 3);

    let hits = s.search_text("fox", 10).unwrap();
    assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![2, 1]);
    assert!(
        hits[0].snippet.contains("<mark>fox</mark>"),
        "{}",
        hits[0].snippet
    );
    assert!(hits[1].snippet.contains("<mark>fox</mark>"));

    // limit
    assert_eq!(s.search_text("fox", 1).unwrap().len(), 1);
    assert!(s.search_text("fox", 0).unwrap().is_empty());
    // all words must match
    let both = s.search_text("fox lazy", 10).unwrap();
    assert_eq!(both.len(), 1);
    assert_eq!(both[0].id, 1);
    // stemming + case
    assert_eq!(s.search_text("JUMPING", 10).unwrap()[0].id, 1);
    // diacritics are folded
    s.index_book_text(3, "Crème brûlée et café").unwrap();
    assert_eq!(s.search_text("creme brulee", 10).unwrap()[0].id, 3);
}

#[test]
fn text_search_survives_hostile_queries() {
    let s = store_with(&[book(1, "a", "x")]);
    s.index_book_text(1, "hello world AND OR NOT").unwrap();
    for q in [
        "",
        "   ",
        "\"",
        "\"hello",
        "hello*",
        "(",
        "NEAR(",
        "a:b",
        "-",
        "AND",
        "'; DROP TABLE books;--",
    ] {
        s.search_text(q, 10)
            .unwrap_or_else(|e| panic!("{q:?}: {e}"));
    }
    assert_eq!(s.search_text("\"hello\" world", 5).unwrap().len(), 1);
    assert_eq!(s.len(), 1);
}

#[test]
fn text_index_is_replaced_and_removed_with_the_book() {
    let s = store_with(&[book(1, "a", "x")]);
    s.index_book_text(1, "first version mentions ostrich")
        .unwrap();
    s.index_book_text(1, "second version mentions emu").unwrap();
    assert!(s.search_text("ostrich", 5).unwrap().is_empty());
    assert_eq!(s.search_text("emu", 5).unwrap().len(), 1);
    assert_eq!(s.indexed_text_count(), 1);
    // updating the metadata keeps the text index
    s.put(book(1, "renamed", "x")).unwrap();
    assert_eq!(s.search_text("emu", 5).unwrap().len(), 1);
    s.remove(1).unwrap();
    assert!(s.search_text("emu", 5).unwrap().is_empty());
    assert_eq!(s.indexed_text_count(), 0);
}

#[test]
fn text_for_unknown_book_is_an_error() {
    let s = SqliteStore::open_in_memory().unwrap();
    let err = s.index_book_text(5, "x").unwrap_err();
    assert!(err.to_string().contains("no such book 5"), "{err}");
    assert!(s
        .index_book_texts(&[(5, "x".into())])
        .unwrap_err()
        .to_string()
        .contains("no such book"));
    assert_eq!(s.indexed_text_count(), 0);
}

#[test]
fn batch_text_indexing_and_unicode_text() {
    let s = store_with(&[book(1, "a", "x"), book(2, "b", "x")]);
    s.index_book_texts(&[(1, "Привет мир".into()), (2, "日本語 テキスト".into())])
        .unwrap();
    assert_eq!(s.search_text("привет", 5).unwrap()[0].id, 1);
    assert_eq!(s.search_text("日本語", 5).unwrap()[0].id, 2);
    s.index_book_texts(&[]).unwrap();
}

// -- equality with the pure-Rust query engine -------------------------------------------------

const WORDS: &[&str] = &[
    "dragon",
    "river",
    "silver",
    "night",
    "empire",
    "garden",
    "storm",
    "shadow",
    "crown",
    "ocean",
    "Winter",
    "SUMMER",
    "ash",
    "ox",
    "tolkien",
    "leguin",
    "pratchett",
    "discworld",
    "fantasy",
    "horror",
    "scifi",
    "history",
    "penguin",
    "tor",
    "orbit",
];

fn phrase(r: &mut Rng, n: usize) -> String {
    (0..n).map(|_| r.pick(WORDS)).collect::<Vec<_>>().join(" ")
}

fn generated_books(n: usize) -> Vec<Book> {
    let mut r = Rng(0x9E3779B97F4A7C15);
    (1..=n as u64)
        .map(|id| {
            let n = 1 + r.below(3);
            let mut b = book(id, &phrase(&mut r, n), &phrase(&mut r, 2));
            if r.below(3) == 0 {
                b.meta.authors.push(phrase(&mut r, 2));
            }
            b.meta.tags = (0..r.below(4)).map(|_| r.pick(WORDS).to_string()).collect();
            if r.below(2) == 0 {
                b.meta.series = phrase(&mut r, 2);
                b.meta.series_index = (1 + r.below(5)) as f64;
            }
            b.meta.publisher = r.pick(WORDS).to_string();
            b.meta.language = r.pick(&["en", "de", "fr"]).to_string();
            b.meta.description = phrase(&mut r, 5);
            b.meta.rating = r.below(11) as u8;
            if r.below(4) == 0 {
                b.meta.identifiers = vec![Identifier {
                    scheme: "isbn".into(),
                    value: format!("97800{}", r.below(100000)),
                    xml_id: None,
                }];
            }
            b.size = (r.below(5_000_000)) as u64;
            b.progress = [0.0, 0.3, 0.99, 1.0][r.below(4)];
            b
        })
        .collect()
}

#[test]
fn search_ids_equals_the_pure_query_engine() {
    let books = generated_books(500);
    let s = store_with(&books);
    let queries = [
        "dragon",
        "DRAGON",
        "storm",
        "tolkien",
        "ash",
        "ox",
        "xyzzy",
        "silver dragon",
        "dragon OR river",
        "-dragon",
        "dragon -river",
        "author:tolkien",
        "author:=\"tolkien silver\"",
        "title:night",
        "tag:fantasy",
        "tag:",
        "tags:=horror",
        "series:discworld",
        "publisher:penguin",
        "language:de",
        "lang:=fr",
        "rating:>=4",
        "rating:3",
        "rating:<2",
        "status:reading",
        "status:finished OR status:unread",
        "size:>1",
        "progress:>50",
        "identifier:isbn:97800",
        "isbn:97800",
        "format:epub",
        "comments:garden",
        "\"winter summer\"",
        "ocean crown shadow",
        "",
        "   ",
        "ünï",
    ];
    assert!(queries.len() >= 30);
    let mut non_trivial = 0;
    for q in queries {
        let expected: Vec<u64> = {
            let query = Query::parse(q);
            books
                .iter()
                .filter(|b| query.matches(b))
                .map(|b| b.id)
                .collect()
        };
        assert_eq!(s.search_ids(q), expected, "query {q:?}");
        if !expected.is_empty() && expected.len() < books.len() {
            non_trivial += 1;
        }
    }
    assert!(
        non_trivial >= 20,
        "queries must select something: {non_trivial}"
    );
}

#[test]
fn fast_path_equals_slow_path_for_many_words() {
    let books = generated_books(300);
    let s = store_with(&books);
    for w in WORDS.iter().filter(|w| is_fast_query(w)) {
        let query = Query::parse(w);
        let expected: Vec<u64> = books
            .iter()
            .filter(|b| query.matches(b))
            .map(|b| b.id)
            .collect();
        assert_eq!(s.search_ids(w), expected, "{w}");
    }
    // substrings inside words are found as well (contains semantics)
    let query = Query::parse("rago");
    let expected: Vec<u64> = books
        .iter()
        .filter(|b| query.matches(b))
        .map(|b| b.id)
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(s.search_ids("rago"), expected);
}

#[test]
fn search_results_follow_edits() {
    let mut books = generated_books(60);
    let s = store_with(&books);
    for b in books.iter_mut().step_by(7) {
        b.meta.title = "Renamed Specimen".into();
        s.put(b.clone()).unwrap();
    }
    s.remove(books[1].id).unwrap();
    books.remove(1);
    for q in ["specimen", "dragon", "tag:fantasy"] {
        let query = Query::parse(q);
        let expected: Vec<u64> = books
            .iter()
            .filter(|b| query.matches(b))
            .map(|b| b.id)
            .collect();
        assert_eq!(s.search_ids(q), expected, "{q}");
    }
    let ids: Vec<u64> = s.ids();
    assert_eq!(ids, books.iter().map(|b| b.id).collect::<Vec<_>>());
}
