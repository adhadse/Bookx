//! Duplicate detection beyond "same title and author" (L7).
//!
//! Two books are duplicates when they share an ISBN, or when their titles (normalised: accents,
//! case, punctuation, leading articles, subtitles) and authors (`Last, First` vs `First Last`,
//! initials) agree closely enough.

use std::collections::HashMap;

use bookx_core::library::Book;
use bookx_core::opf::Metadata;

use crate::authors::{parse_name, same_authors};
use crate::isbn;
use crate::text::{levenshtein_ratio, normalize, title_similarity};

/// Score at or above which two records count as duplicates unless the caller says otherwise.
pub const DEFAULT_THRESHOLD: f64 = 0.88;

const ARTICLES: &[&str] = &[
    "the", "a", "an", "der", "die", "das", "ein", "eine", "le", "la", "les", "l", "el", "los",
    "las", "il", "lo", "un", "une", "una", "het", "een",
];

/// A title split into its comparable parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleKey {
    /// Normalised full title without a leading article.
    pub full: String,
    /// Normalised part before the subtitle separator (`:`, ` - `, ` – `, ` — `).
    pub main: String,
    /// Normalised subtitle, empty when there is none.
    pub sub: String,
}

fn strip_article(norm: &str) -> String {
    // "hobbit the" from the sort form "Hobbit, The" is handled before normalising
    let mut words: Vec<&str> = norm.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() > 1 && ARTICLES.contains(&words[0]) {
        words.remove(0);
    }
    words.join(" ")
}

/// `L'Étranger` → `Étranger`, `d'Artagnan` is left alone.
fn strip_elision(title: &str) -> String {
    let mut chars = title.chars();
    match (chars.next(), chars.next()) {
        (Some('L' | 'l'), Some('\'' | '\u{2019}')) if title.chars().count() > 2 => {
            title.chars().skip(2).collect()
        }
        _ => title.to_string(),
    }
}

fn undo_sort_form(title: &str) -> String {
    let t = title.trim();
    if let Some((head, tail)) = t.rsplit_once(',') {
        let tail_n = normalize(tail);
        if ARTICLES.contains(&tail_n.as_str()) && !head.trim().is_empty() {
            return format!("{} {}", tail.trim(), head.trim());
        }
    }
    t.to_string()
}

fn split_subtitle(title: &str) -> (&str, &str) {
    let mut best: Option<(usize, usize)> = None;
    for sep in [":", " (", " - ", " \u{2013} ", " \u{2014} "] {
        if let Some(i) = title.find(sep) {
            if i > 0 && best.is_none_or(|(b, _)| i < b) {
                best = Some((i, sep.len()));
            }
        }
    }
    match best {
        Some((i, len)) => (&title[..i], &title[i + len..]),
        None => (title, ""),
    }
}

const NUMBER_WORDS: &[&str] = &[
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
];
const ROMAN: &[&str] = &[
    "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv", "xv",
];
const ROMAN_CUES: &[&str] = &[
    "part", "book", "volume", "vol", "chapter", "episode", "tome",
];

/// Writes numbers one way: `two` → `2`, and a Roman numeral after `part`/`book`/`volume` →
/// `Part II` = `part 2`. Input is already normalised (lower case, single spaces).
fn numerals(norm: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for w in norm.split(' ').filter(|w| !w.is_empty()) {
        let after_cue = out.last().is_some_and(|p| ROMAN_CUES.contains(&p.as_str()));
        if let Some(n) = NUMBER_WORDS.iter().position(|x| x == &w) {
            out.push(n.to_string());
        } else if let (true, Some(n)) = (after_cue, ROMAN.iter().position(|x| x == &w)) {
            out.push((n + 1).to_string());
        } else {
            out.push(w.to_string());
        }
    }
    out.join(" ")
}

/// Builds the comparison key of a title.
pub fn title_key(title: &str) -> TitleKey {
    let t = undo_sort_form(title);
    let t = strip_elision(&t);
    let (main, sub) = split_subtitle(&t);
    TitleKey {
        full: numerals(&strip_article(&normalize(&t))),
        main: numerals(&strip_article(&normalize(main))),
        sub: numerals(&normalize(sub)),
    }
}

/// Normalised title without article and punctuation (`"The Hobbit, or There…"` → `"hobbit or there…"`).
pub fn normalize_title(title: &str) -> String {
    title_key(title).full
}

/// Similarity of two titles in 0..=1, forgiving about case, accents, articles and subtitles.
/// A title that equals the other one's main title (the other has a subtitle) scores 0.9.
fn title_score_raw(a: &str, b: &str) -> f64 {
    let (ka, kb) = (title_key(a), title_key(b));
    if ka.full.is_empty() || kb.full.is_empty() {
        return if ka.full.is_empty() && kb.full.is_empty() && a.trim() == b.trim() {
            1.0
        } else {
            0.0
        };
    }
    let full = title_similarity(&ka.full, &kb.full);
    if ka.main.is_empty() || kb.main.is_empty() || (ka.sub.is_empty() && kb.sub.is_empty()) {
        return full;
    }
    let main = title_similarity(&ka.main, &kb.main);
    if main < 0.95 {
        return full;
    }
    let sub = match (ka.sub.is_empty(), kb.sub.is_empty()) {
        (true, _) | (_, true) => 0.5,
        _ => levenshtein_ratio(&ka.sub, &kb.sub),
    };
    full.max(0.8 + 0.2 * sub * main)
}

/// Numbers in a title are part of its identity ("Part 1" vs "Part 2"): when both titles have
/// numbers and they differ, the score is capped at 0.7.
pub fn title_score(a: &str, b: &str) -> f64 {
    let raw = title_score_raw(a, b);
    let numbers = |t: &str| -> Vec<String> {
        let mut n: Vec<String> = normalize(t)
            .split(' ')
            .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
            .map(String::from)
            .collect();
        n.sort();
        n
    };
    let (na, nb) = (numbers(a), numbers(b));
    if !na.is_empty() && !nb.is_empty() && na != nb {
        raw.min(0.7)
    } else {
        raw
    }
}

fn series_conflict(a: &Metadata, b: &Metadata) -> bool {
    !a.series.is_empty()
        && normalize(&a.series) == normalize(&b.series)
        && a.series_index > 0.0
        && b.series_index > 0.0
        && (a.series_index - b.series_index).abs() > 1e-9
}

/// Whether two records share a valid ISBN (ISBN-10 and ISBN-13 forms are equal).
pub fn share_isbn(a: &Metadata, b: &Metadata) -> bool {
    let ia = isbns_of(a);
    let ib = isbns_of(b);
    ia.iter().any(|x| ib.contains(x))
}

fn isbns_of(m: &Metadata) -> Vec<String> {
    m.identifiers
        .iter()
        .filter(|i| i.scheme.to_lowercase().starts_with("isbn") || i.scheme.is_empty())
        .filter_map(|i| isbn::canonical(&i.value))
        .collect()
}

/// Duplicate score in 0..=1. Symmetric; `1.0` for identical records and for equal ISBNs.
pub fn similarity(a: &Metadata, b: &Metadata) -> f64 {
    if share_isbn(a, b) {
        return 1.0;
    }
    if series_conflict(a, b) {
        return 0.0;
    }
    let t = title_score(&a.title, &b.title);
    let factor = match (a.authors.is_empty(), b.authors.is_empty()) {
        (true, true) => 1.0,
        (false, false) => {
            if same_authors(&a.authors, &b.authors) {
                1.0
            } else {
                0.5
            }
        }
        _ => 0.9,
    };
    t * factor
}

/// `similarity(a, b) >= threshold`.
pub fn is_duplicate(a: &Metadata, b: &Metadata, threshold: f64) -> bool {
    similarity(a, b) >= threshold
}

/// Books that are probably the same work, ids ascending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateSet {
    pub ids: Vec<u64>,
}

struct Dsu(Vec<usize>);

impl Dsu {
    fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut c = x;
        while self.0[c] != r {
            let n = self.0[c];
            self.0[c] = r;
            c = n;
        }
        r
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[rb.max(ra)] = rb.min(ra);
        }
    }
}

/// What a cheap pre-check needs to rule a pair out without scoring its titles: the length and a
/// character histogram of the normalised title. See [`TitlePre::cannot_reach`].
struct TitlePre {
    len: usize,
    hist: [u16; 40],
    /// No subtitle and a non-empty title: the title score is then exactly the full-title similarity.
    plain: bool,
}

impl TitlePre {
    fn new(title: &str) -> TitlePre {
        let key = title_key(title);
        let norm = normalize(&key.full);
        let mut hist = [0u16; 40];
        let mut len = 0;
        for c in norm.chars() {
            len += 1;
            let slot = match c {
                'a'..='z' => c as usize - 'a' as usize,
                '0'..='9' => 26 + c as usize - '0' as usize,
                _ => 36 + (c as usize % 4),
            };
            hist[slot] = hist[slot].saturating_add(1);
        }
        TitlePre {
            len,
            hist,
            plain: key.sub.is_empty() && len > 0,
        }
    }

    /// True when the title similarity of the two books is certainly below `threshold`. It bounds the
    /// scores [`title_similarity`] takes the best of from above using only the characters the two
    /// titles share (a multiset overlap `c`): Levenshtein ratio is at most `c / max_len`, and
    /// Jaro-Winkler at most `jaro + 0.4 (1 - jaro)` with `jaro <= (c/la + c/lb + 1) / 3`, scaled the
    /// way `title_similarity` scales it. Titles with a subtitle are never ruled out here.
    fn cannot_reach(&self, other: &TitlePre, threshold: f64) -> bool {
        if !(self.plain && other.plain) {
            return false;
        }
        let common: usize = self
            .hist
            .iter()
            .zip(other.hist.iter())
            .map(|(a, b)| *a.min(b) as usize)
            .sum();
        let (la, lb, c) = (self.len as f64, other.len as f64, common as f64);
        let lev = c / la.max(lb);
        let jaro = ((c / la + c / lb + 1.0) / 3.0).min(1.0);
        let jw = (jaro + 0.4 * (1.0 - jaro)).min(1.0);
        let jw = if jw >= 0.85 { jw * 0.98 } else { jw * 0.6 };
        lev.max(jw) < threshold
    }
}

/// Groups `books` into duplicate sets (transitively). Only books that share a block are compared:
/// an ISBN, a main title, or the surname of the first author, so large libraries stay fast.
pub fn find_duplicates(books: &[Book], threshold: f64) -> Vec<DuplicateSet> {
    let mut blocks: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, b) in books.iter().enumerate() {
        let m = &b.meta;
        for n in isbns_of(m) {
            blocks.entry(format!("i:{n}")).or_default().push(i);
        }
        let k = title_key(&m.title);
        if !k.main.is_empty() {
            blocks.entry(format!("t:{}", k.main)).or_default().push(i);
        }
        let last = m
            .authors
            .first()
            .map(|a| normalize(&parse_name(a).last))
            .unwrap_or_default();
        blocks.entry(format!("a:{last}")).or_default().push(i);
    }
    let mut dsu = Dsu((0..books.len()).collect());
    // an ISBN match scores 1.0 whatever the titles say, so the title pre-check only applies to
    // books that share no ISBN
    let pre: Vec<TitlePre> = books.iter().map(|b| TitlePre::new(&b.meta.title)).collect();
    let isbn_sets: Vec<Vec<String>> = books.iter().map(|b| isbns_of(&b.meta)).collect();
    for members in blocks.values() {
        for (x, &i) in members.iter().enumerate() {
            for &j in &members[x + 1..] {
                if dsu.find(i) == dsu.find(j) {
                    continue;
                }
                let shares_isbn = isbn_sets[i].iter().any(|n| isbn_sets[j].contains(n));
                if !shares_isbn && pre[i].cannot_reach(&pre[j], threshold) {
                    continue;
                }
                if is_duplicate(&books[i].meta, &books[j].meta, threshold) {
                    dsu.union(i, j);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<u64>> = HashMap::new();
    for (i, b) in books.iter().enumerate() {
        let r = dsu.find(i);
        groups.entry(r).or_default().push(b.id);
    }
    let mut out: Vec<DuplicateSet> = groups
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut ids| {
            ids.sort_unstable();
            DuplicateSet { ids }
        })
        .collect();
    out.sort_by_key(|s| s.ids[0]);
    out
}

fn first_non_empty<'a>(items: impl Iterator<Item = &'a str>) -> String {
    items
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Combines records of the same book: `primary` wins every field it has; empty fields are filled
/// from `others` in order; tags and identifiers are unioned (case-insensitively).
pub fn merge_metadata(primary: &Metadata, others: &[Metadata]) -> Metadata {
    let all: Vec<&Metadata> = std::iter::once(primary).chain(others.iter()).collect();
    let mut out = Metadata {
        title: first_non_empty(all.iter().map(|m| m.title.as_str())),
        title_sort: first_non_empty(all.iter().map(|m| m.title_sort.as_str())),
        author_sort: first_non_empty(all.iter().map(|m| m.author_sort.as_str())),
        publisher: first_non_empty(all.iter().map(|m| m.publisher.as_str())),
        language: first_non_empty(all.iter().map(|m| m.language.as_str())),
        description: first_non_empty(all.iter().map(|m| m.description.as_str())),
        published: first_non_empty(all.iter().map(|m| m.published.as_str())),
        rights: first_non_empty(all.iter().map(|m| m.rights.as_str())),
        authors: all
            .iter()
            .find(|m| m.authors.iter().any(|a| !a.trim().is_empty()))
            .map(|m| m.authors.clone())
            .unwrap_or_default(),
        rating: all.iter().map(|m| m.rating).find(|r| *r > 0).unwrap_or(0),
        ..Default::default()
    };
    // series and its index travel together
    if let Some(m) = all.iter().find(|m| !m.series.trim().is_empty()) {
        out.series = m.series.trim().to_string();
        out.series_index = m.series_index;
    } else {
        out.series_index = all
            .iter()
            .map(|m| m.series_index)
            .find(|i| *i > 0.0)
            .unwrap_or(0.0);
    }
    for m in &all {
        for t in &m.tags {
            let t = t.trim();
            if !t.is_empty() && !out.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                out.tags.push(t.to_string());
            }
        }
        for id in &m.identifiers {
            let dup = out.identifiers.iter().any(|x| {
                x.scheme.eq_ignore_ascii_case(&id.scheme) && x.value.trim() == id.value.trim()
            });
            if !dup && !id.value.trim().is_empty() {
                out.identifiers.push(id.clone());
            }
        }
    }
    // only the primary's xml ids are meaningful in the merged record
    for id in &mut out.identifiers {
        if !primary.identifiers.contains(id) {
            id.xml_id = None;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::opf::Identifier;
    use chrono::Utc;

    fn md(title: &str, authors: &[&str]) -> Metadata {
        Metadata {
            title: title.into(),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn with_isbn(mut m: Metadata, i: &str) -> Metadata {
        m.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: i.into(),
            xml_id: None,
        });
        m
    }

    fn book(id: u64, m: Metadata) -> Book {
        Book {
            id,
            file: format!("b{id}.epub"),
            cover: None,
            meta: m,
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
            source_ids: Vec::new(),
        }
    }

    #[test]
    fn title_keys_drop_articles_punctuation_and_split_subtitles() {
        let k = title_key("The Hobbit: Or, There and Back Again");
        assert_eq!(k.main, "hobbit");
        assert_eq!(k.sub, "or there and back again");
        assert!(k.full.starts_with("hobbit or"));
        assert_eq!(normalize_title("Les Misérables"), "miserables");
        assert_eq!(normalize_title("Hobbit, The"), "hobbit");
        assert_eq!(normalize_title("A Game of Thrones"), "game of thrones");
        assert_eq!(normalize_title("The"), "the", "a lone article stays");
        assert_eq!(normalize_title("L'Étranger"), "etranger");
        assert_eq!(title_key("Dune - Messiah").sub, "messiah");
        assert_eq!(title_key("Dune — Messiah").main, "dune");
        assert_eq!(title_key("").full, "");
        assert_eq!(title_key("Colon:").sub, "");
        assert_eq!(title_key("The Hobbit (Illustrated)").main, "hobbit");
        assert_eq!(normalize_title("d'Artagnan"), "dartagnan");
    }

    #[test]
    fn number_words_and_numerals_compare_equal() {
        assert_eq!(title_key("Book Two").full, title_key("Book 2").full);
        assert_eq!(
            title_key("Part II: The Return").main,
            title_key("Part 2 - The Return").main
        );
        assert_eq!(
            title_key("Fahrenheit Four Fifty-One").full,
            "fahrenheit 4 fifty 1"
        );
        assert!(title_score("The Two Towers", "The 2 Towers") >= 0.99);
        // a lone "I" or "V" is not a number: only after part/book/volume
        assert_eq!(title_key("I Robot").full, "i robot");
        assert_ne!(title_key("Part Three").full, title_key("Part Two").full);
    }

    #[test]
    fn title_score_cases() {
        assert_eq!(title_score("The Hobbit", "Hobbit, The"), 1.0);
        assert_eq!(title_score("Les Misérables", "les miserables"), 1.0);
        assert!(title_score("The Hobbit", "The Hobbit: Or There and Back Again") >= 0.9);
        assert!(title_score("Dune: Messiah", "Dune: Children of Dune") < 0.88);
        assert!(title_score("Dune", "Neuromancer") < 0.6);
        assert_eq!(title_score("", ""), 1.0);
        assert!(
            title_score(
                "Book number 5 of the grand collection",
                "Book number 15 of the grand collection"
            ) <= 0.7
        );
        assert!(title_score("Part 1: The Beginning", "Part 2: The Beginning") <= 0.7);
        assert!(
            title_score("Hobbit 1937 Edition", "Hobbit Edition") > 0.8,
            "numbers on one side only do not conflict"
        );
        assert_eq!(title_score("Catch 22", "catch-22"), 1.0);
        assert_eq!(title_score("", "x"), 0.0);
        assert_eq!(title_score("!!", "??"), 0.0);
        assert!(
            title_score("The Lord of the Rings", "Lord of the Ring") > 0.9,
            "typo tolerant"
        );
    }

    #[test]
    fn isbn_equality_wins_over_everything() {
        let a = with_isbn(md("Totally Different", &["X Y"]), "0-306-40615-2");
        let b = with_isbn(md("Another One", &["Z W"]), "978-0-306-40615-7");
        assert_eq!(similarity(&a, &b), 1.0);
        assert!(share_isbn(&a, &b));
        let c = with_isbn(md("Totally Different", &["X Y"]), "9783161484100");
        assert!(!share_isbn(&a, &c));
        let bad = with_isbn(md("Q", &[]), "123");
        let bad2 = with_isbn(md("R", &[]), "123");
        assert!(!share_isbn(&bad, &bad2), "invalid ISBNs never match");
    }

    #[test]
    fn author_forms_match() {
        let a = md("The Hobbit", &["J.R.R. Tolkien"]);
        let b = md("Hobbit, The", &["Tolkien, John Ronald Reuel"]);
        assert!(is_duplicate(&a, &b, DEFAULT_THRESHOLD));
        let c = md("The Hobbit", &["C. S. Lewis"]);
        assert!(!is_duplicate(&a, &c, DEFAULT_THRESHOLD));
    }

    #[test]
    fn missing_authors_are_weaker_evidence() {
        let a = md("Dune", &["Frank Herbert"]);
        let b = md("Dune", &[]);
        let s = similarity(&a, &b);
        assert!((DEFAULT_THRESHOLD..1.0).contains(&s), "{s}");
        assert_eq!(similarity(&md("Dune", &[]), &md("Dune", &[])), 1.0);
    }

    #[test]
    fn different_volumes_of_one_series_are_not_duplicates() {
        let mut a = md("Wee Free Men", &["Terry Pratchett"]);
        a.series = "Discworld".into();
        a.series_index = 30.0;
        let mut b = a.clone();
        b.series_index = 31.0;
        assert_eq!(similarity(&a, &b), 0.0);
        b.series_index = 30.0;
        assert_eq!(similarity(&a, &b), 1.0);
    }

    #[test]
    fn find_duplicates_groups_transitively() {
        let books = vec![
            book(1, md("The Hobbit", &["J.R.R. Tolkien"])),
            book(2, md("Hobbit, The", &["Tolkien, J. R. R."])),
            book(3, md("Dune", &["Frank Herbert"])),
            book(
                4,
                with_isbn(md("A Different Title", &["Someone"]), "0306406152"),
            ),
            book(
                5,
                with_isbn(md("Unrelated", &["Other"]), "978-0-306-40615-7"),
            ),
            book(
                6,
                md("The Hobbit (Illustrated)", &["John Ronald Reuel Tolkien"]),
            ),
            book(7, md("Neuromancer", &["William Gibson"])),
        ];
        let sets = find_duplicates(&books, DEFAULT_THRESHOLD);
        assert_eq!(
            sets,
            vec![
                DuplicateSet { ids: vec![1, 2, 6] },
                DuplicateSet { ids: vec![4, 5] }
            ]
        );
    }

    #[test]
    fn find_duplicates_edge_cases() {
        assert!(find_duplicates(&[], 0.9).is_empty());
        assert!(find_duplicates(&[book(1, md("A", &["B"]))], 0.9).is_empty());
        let none = vec![book(1, md("", &[])), book(2, md("x", &[]))];
        assert!(find_duplicates(&none, 0.9).is_empty());
        let same = vec![book(9, md("Same", &["A B"])), book(3, md("Same", &["A B"]))];
        assert_eq!(find_duplicates(&same, 0.9)[0].ids, vec![3, 9], "ids sorted");
    }

    #[test]
    fn threshold_is_respected() {
        let a = md("The Lord of the Rings", &["J. R. R. Tolkien"]);
        let b = md("Lord of the Ring", &["J. R. R. Tolkien"]);
        assert!(is_duplicate(&a, &b, 0.9));
        assert!(!is_duplicate(&a, &b, 1.0));
    }

    #[test]
    fn merge_fills_gaps_and_unions() {
        let mut p = md("Dune", &[]);
        p.tags = vec!["SciFi".into()];
        p.identifiers = vec![Identifier {
            scheme: "uuid".into(),
            value: "u1".into(),
            xml_id: Some("bookid".into()),
        }];
        let mut o1 = md("Dune: Special", &["Frank Herbert"]);
        o1.publisher = "Ace".into();
        o1.tags = vec!["scifi".into(), "Classic".into()];
        o1.series = "Dune Chronicles".into();
        o1.series_index = 1.0;
        o1.rating = 8;
        o1.identifiers = vec![
            Identifier {
                scheme: "ISBN".into(),
                value: "0441172717".into(),
                xml_id: Some("x".into()),
            },
            Identifier {
                scheme: "uuid".into(),
                value: "u1".into(),
                xml_id: None,
            },
        ];
        let mut o2 = md("", &["Other"]);
        o2.description = "Desert planet".into();
        o2.language = "en".into();
        o2.published = "1965".into();
        let m = merge_metadata(&p, &[o1, o2]);
        assert_eq!(m.title, "Dune");
        assert_eq!(m.authors, vec!["Frank Herbert"]);
        assert_eq!(m.publisher, "Ace");
        assert_eq!(m.tags, vec!["SciFi", "Classic"]);
        assert_eq!(
            (m.series.as_str(), m.series_index),
            ("Dune Chronicles", 1.0)
        );
        assert_eq!(m.rating, 8);
        assert_eq!(m.description, "Desert planet");
        assert_eq!(m.published, "1965");
        assert_eq!(m.identifiers.len(), 2);
        assert_eq!(m.identifiers[0].xml_id.as_deref(), Some("bookid"));
        assert_eq!(m.identifiers[1].xml_id, None, "foreign xml ids are dropped");
    }

    #[test]
    fn merge_primary_wins_and_handles_empties() {
        let mut p = md("A", &["P"]);
        p.series = "S".into();
        p.series_index = 2.0;
        let mut o = md("B", &["Q"]);
        o.series = "T".into();
        o.series_index = 9.0;
        let m = merge_metadata(&p, &[o]);
        assert_eq!((m.title.as_str(), m.authors[0].as_str()), ("A", "P"));
        assert_eq!((m.series.as_str(), m.series_index), ("S", 2.0));
        let empty = merge_metadata(&Metadata::default(), &[]);
        assert_eq!(empty, Metadata::default());
        let idx_only = Metadata {
            series_index: 4.0,
            ..Default::default()
        };
        assert_eq!(
            merge_metadata(&Metadata::default(), &[idx_only]).series_index,
            4.0
        );
        let blank_authors = merge_metadata(&md("T", &["  "]), &[md("T", &["Real"])]);
        assert_eq!(blank_authors.authors, vec!["Real"]);
    }

    // --- property tests with a deterministic PRNG -------------------------------------------

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[(self.next() % items.len() as u64) as usize]
        }
    }

    fn random_meta(r: &mut Rng) -> Metadata {
        const W: &[&str] = &[
            "The", "Dune", "Hobbit", "Café", "Straße", "of", "Rings", "Lord", "日本", "A", "!",
        ];
        const F: &[&str] = &["J.R.R.", "John", "Ada", "Émile", "K.", "Ursula"];
        const L: &[&str] = &["Tolkien", "Lovelace", "Le Guin", "Zola", "Smith"];
        let n = 1 + r.next() % 4;
        let title: Vec<&str> = (0..n).map(|_| r.pick(W)).collect();
        let mut m = Metadata {
            title: title.join(" "),
            ..Default::default()
        };
        for _ in 0..(r.next() % 3) {
            let (f, l) = (r.pick(F), r.pick(L));
            m.authors.push(if r.next() % 2 == 0 {
                format!("{f} {l}")
            } else {
                format!("{l}, {f}")
            });
        }
        if r.next() % 5 == 0 {
            m.series = r.pick(W).to_string();
            m.series_index = (r.next() % 3) as f64;
        }
        m
    }

    #[test]
    fn property_similarity_is_symmetric_and_bounded() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..2000 {
            let (a, b) = (random_meta(&mut r), random_meta(&mut r));
            let (ab, ba) = (similarity(&a, &b), similarity(&b, &a));
            assert!((ab - ba).abs() < 1e-12, "{a:?} vs {b:?}: {ab} != {ba}");
            assert!((0.0..=1.0 + 1e-12).contains(&ab), "{ab}");
        }
    }

    #[test]
    fn property_similarity_is_reflexive() {
        let mut r = Rng(42);
        for _ in 0..2000 {
            let a = random_meta(&mut r);
            assert_eq!(similarity(&a, &a.clone()), 1.0, "{a:?}");
            assert!(is_duplicate(&a, &a.clone(), 1.0));
        }
    }

    #[test]
    fn property_find_duplicates_is_order_independent() {
        let mut r = Rng(7);
        let books: Vec<Book> = (0..60).map(|i| book(i, random_meta(&mut r))).collect();
        let mut rev = books.clone();
        rev.reverse();
        assert_eq!(find_duplicates(&books, 0.9), find_duplicates(&rev, 0.9));
    }

    #[test]
    fn property_merge_is_idempotent_on_self() {
        let mut r = Rng(99);
        for _ in 0..200 {
            let a = random_meta(&mut r);
            let m = merge_metadata(&a, std::slice::from_ref(&a));
            assert_eq!(m.title, a.title.trim());
            assert_eq!(m.authors, a.authors);
            assert_eq!(m.tags, a.tags);
        }
    }

    /// The title pre-check only ever skips pairs that the full scoring would reject too.
    #[test]
    fn the_title_pre_check_never_hides_a_duplicate() {
        let base = [
            "The Hobbit",
            "Hobbit, The",
            "The Hobbit!",
            "Hobbit",
            "L'Étranger",
            "Etranger",
            "Dune",
            "Dune Messiah",
            "Dunes",
            "War and Peace",
            "War & Peace",
            "Wars and Peace",
            "A Tale of Two Cities",
            "Tale of Two Cities",
            "Tales of Two Cities",
            "Neuromancer",
            "Neuromancers",
            "Neuromancer: A Novel",
            "Harry Potter 1",
            "Harry Potter 2",
            "Zzzz",
            "aaaa",
            "ab",
            "ba",
            "",
        ];
        let pres: Vec<TitlePre> = base.iter().map(|t| TitlePre::new(t)).collect();
        for threshold in [0.6, DEFAULT_THRESHOLD, 0.95] {
            for (i, a) in base.iter().enumerate() {
                for (j, b) in base.iter().enumerate() {
                    if pres[i].cannot_reach(&pres[j], threshold) {
                        let score = title_score(a, b);
                        assert!(
                            score < threshold,
                            "{a:?} vs {b:?}: pre-check says unreachable but the score is {score}"
                        );
                    }
                }
            }
        }
        // and it does rule out clearly different titles
        assert!(
            TitlePre::new("Zzzz Qqqq").cannot_reach(&TitlePre::new("Hobbit"), DEFAULT_THRESHOLD)
        );
        assert!(
            !TitlePre::new("The Hobbit")
                .cannot_reach(&TitlePre::new("Hobbit, The"), DEFAULT_THRESHOLD)
        );
    }

    #[test]
    fn find_duplicates_agrees_with_pairwise_scoring() {
        let titles = [
            "The Hobbit",
            "Hobbit, The",
            "The Hobbit: There and Back Again",
            "Dune",
            "Dune Messiah",
            "Dune!",
            "War and Peace",
            "War & Peace",
        ];
        let books: Vec<Book> = titles
            .iter()
            .enumerate()
            .map(|(i, t)| book(i as u64 + 1, md(t, &["Some Author"])))
            .collect();
        let found = find_duplicates(&books, DEFAULT_THRESHOLD);
        // reference: every pair scored on its own, merged transitively
        let mut group: Vec<usize> = (0..books.len()).collect();
        for i in 0..books.len() {
            for j in i + 1..books.len() {
                if is_duplicate(&books[i].meta, &books[j].meta, DEFAULT_THRESHOLD) {
                    let (gi, gj) = (group[i], group[j]);
                    for g in group.iter_mut() {
                        if *g == gj {
                            *g = gi;
                        }
                    }
                }
            }
        }
        let mut expected: Vec<Vec<u64>> = Vec::new();
        for g in 0..books.len() {
            let ids: Vec<u64> = (0..books.len())
                .filter(|&i| group[i] == g)
                .map(|i| i as u64 + 1)
                .collect();
            if ids.len() > 1 {
                expected.push(ids);
            }
        }
        let got: Vec<Vec<u64>> = found.into_iter().map(|s| s.ids).collect();
        assert_eq!(got, expected);
    }
}
