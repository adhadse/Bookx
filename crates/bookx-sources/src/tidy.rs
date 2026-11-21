//! Tidying author and series names, and series numbers (roadmap PP-G-3).
//!
//! * `Herbert, Frank`, `frank herbert` and `Frank Herbert` become one spelling (the most common one).
//! * `The Dune Chronicles`, `Dune Chronicles` and `Dune Chronicles Series` become one series.
//! * Series numbers are read from `1`, `01`, `I`, `Book Two`, `#3`, `3rd`, `vol. 4`.
//!
//! [`plan`] only describes the changes; [`apply`] writes them through the library, so each one is
//! an undoable edit.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use bookx_core::library::{Book, Library};

use crate::authors::{author_sort, clean_author, to_first_last};
use crate::text::normalize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Authors,
    Series,
}

/// One proposed change to a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: u64,
    pub title: String,
    pub field: Field,
    pub from: String,
    pub to: String,
}

const CUES: &[&str] = &[
    "book", "vol", "vol.", "volume", "part", "no", "no.", "number", "nr", "nr.", "tome", "#",
];
const WORDS: &[&str] = &[
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
const ORDINALS: &[&str] = &[
    "zeroth", "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
    "tenth",
];

fn roman(s: &str) -> Option<u32> {
    let s = s.to_ascii_uppercase();
    if s.is_empty() || s.len() > 6 || !s.chars().all(|c| "IVXL".contains(c)) {
        return None;
    }
    let val = |c: char| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        _ => 50,
    };
    let chars: Vec<u32> = s.chars().map(val).collect();
    let mut total = 0;
    for (i, v) in chars.iter().enumerate() {
        if chars.get(i + 1).is_some_and(|n| n > v) {
            total -= *v as i64;
        } else {
            total += *v as i64;
        }
    }
    let n = u32::try_from(total).ok()?;
    // reject things like "IIII" or "VX" that are not numerals
    (n > 0 && roman_string(n) == s).then_some(n)
}

fn roman_string(mut n: u32) -> String {
    let table = [
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (v, s) in table {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// Reads a series number from the many ways people write it. `None` when there is no number.
pub fn parse_series_index(text: &str) -> Option<f64> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let tokens: Vec<String> = t
        .split(|c: char| c.is_whitespace() || c == ':' || c == '(' || c == ')')
        .flat_map(|w| match w.strip_prefix('#') {
            Some(rest) if !rest.is_empty() => vec!["#".to_string(), rest.to_string()],
            _ => vec![w.to_string()],
        })
        .filter(|w| !w.is_empty())
        .collect();
    for (i, w) in tokens.iter().enumerate() {
        let w = w.trim_matches(|c: char| c == ',' || c == ';');
        let digits = w.trim_end_matches(|c: char| c.is_alphabetic());
        let suffix = &w[digits.len()..];
        if !digits.is_empty() && matches!(suffix, "" | "st" | "nd" | "rd" | "th") {
            if let Ok(v) = digits.replace(',', ".").parse::<f64>() {
                if v.is_finite() && v >= 0.0 {
                    return Some(v);
                }
            }
        }
        if let Some(n) = WORDS.iter().position(|x| x == &w) {
            return Some(n as f64);
        }
        if let Some(n) = ORDINALS.iter().position(|x| x == &w) {
            return Some(n as f64);
        }
        let cued = i > 0 && CUES.contains(&tokens[i - 1].as_str());
        if tokens.len() == 1 || cued {
            if let Some(n) = roman(w) {
                return Some(n as f64);
            }
        }
    }
    None
}

fn title_case_word(w: &str) -> String {
    let mut cs = w.chars();
    match cs.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// One author in the form `First Last`.
pub fn tidy_author(name: &str) -> String {
    let s = to_first_last(&clean_author(name));
    let letters: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    let all_lower = !letters.is_empty() && letters.iter().all(|c| c.is_lowercase());
    let all_upper = letters.len() > 3 && letters.iter().all(|c| c.is_uppercase());
    if all_lower || all_upper {
        return s
            .split(' ')
            .map(|w| {
                if matches!(
                    w,
                    "van" | "von" | "de" | "da" | "del" | "der" | "di" | "la" | "le"
                ) && !all_upper
                {
                    w.to_string()
                } else {
                    title_case_word(w)
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    s
}

/// What makes two series names the same: case, accents, a leading article and a trailing
/// "series" are ignored.
pub fn series_key(name: &str) -> String {
    let mut words: Vec<String> = normalize(name).split(' ').map(str::to_string).collect();
    if words.len() > 1 && matches!(words[0].as_str(), "the" | "a" | "an") {
        words.remove(0);
    }
    if words.len() > 1
        && matches!(
            words.last().map(String::as_str),
            Some("series" | "saga" | "cycle" | "trilogy")
        )
    {
        words.pop();
    }
    words.join(" ")
}

/// The spelling with the most votes; ties go to the one that sorts first.
fn most_common(forms: &[String]) -> String {
    let mut votes: BTreeMap<&String, usize> = BTreeMap::new();
    for f in forms {
        *votes.entry(f).or_default() += 1;
    }
    votes
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(f, _)| f.clone())
        .unwrap_or_default()
}

/// The changes that would tidy the names of `books`.
pub fn plan(books: &[Book]) -> Vec<Change> {
    let mut out = Vec::new();

    // authors: tidy each, then give equal people one spelling
    let tidy: Vec<Vec<String>> = books
        .iter()
        .map(|b| {
            let mut v: Vec<String> = Vec::new();
            for a in &b.meta.authors {
                let t = tidy_author(a);
                if !t.is_empty() && !v.contains(&t) {
                    v.push(t);
                }
            }
            v
        })
        .collect();
    let mut forms: HashMap<String, Vec<String>> = HashMap::new();
    for names in &tidy {
        for n in names {
            forms.entry(normalize(n)).or_default().push(n.clone());
        }
    }
    let canon: HashMap<String, String> = forms
        .iter()
        .map(|(k, v)| (k.clone(), most_common(v)))
        .collect();
    for (b, names) in books.iter().zip(&tidy) {
        if b.meta.authors.is_empty() {
            continue;
        }
        let mut new: Vec<String> = Vec::new();
        for n in names {
            let c = canon
                .get(&normalize(n))
                .cloned()
                .unwrap_or_else(|| n.clone());
            if !new.contains(&c) {
                new.push(c);
            }
        }
        if new != b.meta.authors {
            out.push(Change {
                id: b.id,
                title: b.meta.title.clone(),
                field: Field::Authors,
                from: b.meta.authors.join(" & "),
                to: new.join(" & "),
            });
        }
    }

    // series: spaces collapsed, then one spelling per key
    let mut series_forms: HashMap<String, Vec<String>> = HashMap::new();
    let cleaned: Vec<String> = books
        .iter()
        .map(|b| {
            b.meta
                .series
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    for s in cleaned.iter().filter(|s| !s.is_empty()) {
        series_forms
            .entry(series_key(s))
            .or_default()
            .push(s.clone());
    }
    let series_canon: HashMap<String, String> = series_forms
        .iter()
        .map(|(k, v)| (k.clone(), most_common(v)))
        .collect();
    for (b, s) in books.iter().zip(&cleaned) {
        if s.is_empty() {
            continue;
        }
        let to = series_canon
            .get(&series_key(s))
            .cloned()
            .unwrap_or_else(|| s.clone());
        if to != b.meta.series {
            out.push(Change {
                id: b.id,
                title: b.meta.title.clone(),
                field: Field::Series,
                from: b.meta.series.clone(),
                to,
            });
        }
    }
    out
}

/// Writes the changes (one undoable edit per book). Returns the number of books changed.
pub fn apply(lib: &mut Library, changes: &[Change]) -> Result<usize> {
    let mut ids: Vec<u64> = changes.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut n = 0;
    for id in ids {
        let Some(b) = lib.get(id) else { continue };
        let mut m = b.meta.clone();
        for c in changes.iter().filter(|c| c.id == id) {
            match c.field {
                Field::Authors => {
                    m.authors = c.to.split(" & ").map(str::to_string).collect();
                    m.author_sort = author_sort(&m.authors);
                }
                Field::Series => m.series = c.to.clone(),
            }
        }
        if m != b.meta {
            lib.update_metadata(id, m)?;
            n += 1;
        }
    }
    Ok(n)
}

/// A book in a series.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesEntry {
    pub id: u64,
    pub index: f64,
    pub title: String,
}

/// A series with its books in order and the numbers that are missing in between.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesGroup {
    pub name: String,
    pub entries: Vec<SeriesEntry>,
    pub missing: Vec<u32>,
}

/// All series of the library, sorted by name, each with its books in reading order.
pub fn series_overview(books: &[Book]) -> Vec<SeriesGroup> {
    let mut groups: BTreeMap<String, SeriesGroup> = BTreeMap::new();
    for b in books.iter().filter(|b| !b.meta.series.trim().is_empty()) {
        let g = groups
            .entry(series_key(&b.meta.series))
            .or_insert_with(|| SeriesGroup {
                name: b.meta.series.trim().to_string(),
                entries: Vec::new(),
                missing: Vec::new(),
            });
        g.entries.push(SeriesEntry {
            id: b.id,
            index: b.meta.series_index,
            title: b.meta.title.clone(),
        });
    }
    let mut out: Vec<SeriesGroup> = groups.into_values().collect();
    for g in &mut out {
        g.entries
            .sort_by(|a, b| a.index.total_cmp(&b.index).then(a.id.cmp(&b.id)));
        let numbered: Vec<u32> = g
            .entries
            .iter()
            .filter(|e| e.index >= 1.0 && e.index.fract() == 0.0 && e.index <= 200.0)
            .map(|e| e.index as u32)
            .collect();
        if let Some(max) = numbered.iter().max() {
            g.missing = (1..*max).filter(|n| !numbered.contains(n)).collect();
        }
    }
    out.sort_by_key(|g| normalize(&g.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::opf::Metadata;
    use chrono::Utc;

    fn book(id: u64, title: &str, authors: &[&str], series: &str, idx: f64) -> Book {
        Book {
            id,
            file: format!("{id}.epub"),
            cover: None,
            meta: Metadata {
                title: title.into(),
                authors: authors.iter().map(|s| s.to_string()).collect(),
                series: series.into(),
                series_index: idx,
                ..Default::default()
            },
            added: Utc::now(),
            modified: Utc::now(),
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

    #[test]
    fn series_numbers_are_read_in_every_style() {
        for (text, want) in [
            ("1", 1.0),
            ("01", 1.0),
            ("1.0", 1.0),
            ("2.5", 2.5),
            ("I", 1.0),
            ("iv", 4.0),
            ("XII", 12.0),
            ("Book Two", 2.0),
            ("Book III", 3.0),
            ("#3", 3.0),
            ("# 4", 4.0),
            ("vol. 7", 7.0),
            ("Volume 10", 10.0),
            ("3rd", 3.0),
            ("second", 2.0),
            ("2 of 5", 2.0),
            ("Part IX", 9.0),
            (" 12 ", 12.0),
        ] {
            assert_eq!(parse_series_index(text), Some(want), "{text:?}");
        }
        for text in ["", "   ", "none", "Book", "IIII", "I am here", "abc"] {
            assert_eq!(parse_series_index(text), None, "{text:?}");
        }
    }

    #[test]
    fn author_names_are_put_in_first_last_order() {
        assert_eq!(tidy_author("Herbert, Frank"), "Frank Herbert");
        assert_eq!(tidy_author("  Frank   Herbert "), "Frank Herbert");
        assert_eq!(tidy_author("frank herbert"), "Frank Herbert");
        assert_eq!(tidy_author("FRANK HERBERT"), "Frank Herbert");
        assert_eq!(tidy_author("ludwig van beethoven"), "Ludwig van Beethoven");
        assert_eq!(tidy_author("Tolkien, J.R.R."), "J.R.R. Tolkien");
        assert_eq!(tidy_author("Smith, John, Jr."), "John Smith Jr.");
        assert_eq!(tidy_author("Plato"), "Plato");
        assert_eq!(tidy_author(""), "");
    }

    #[test]
    fn series_keys_ignore_article_case_and_the_word_series() {
        assert_eq!(
            series_key("The Dune Chronicles"),
            series_key("dune chronicles")
        );
        assert_eq!(
            series_key("Dune Chronicles Series"),
            series_key("Dune Chronicles")
        );
        assert_ne!(series_key("Dune"), series_key("Dune Messiah"));
        assert_eq!(
            series_key("Series"),
            "series",
            "a series called Series keeps its name"
        );
    }

    #[test]
    fn the_plan_unifies_spellings_to_the_most_common_one() {
        let books = vec![
            book(1, "Dune", &["Frank Herbert"], "Dune Chronicles", 1.0),
            book(
                2,
                "Messiah",
                &["Herbert, Frank"],
                "The Dune Chronicles",
                2.0,
            ),
            book(3, "Children", &["frank herbert"], "Dune Chronicles", 3.0),
            book(4, "Other", &["Jane Austen"], "", 0.0),
        ];
        let c = plan(&books);
        let authors: Vec<_> = c.iter().filter(|c| c.field == Field::Authors).collect();
        assert_eq!(authors.len(), 2);
        assert!(authors.iter().all(|c| c.to == "Frank Herbert"));
        let series: Vec<_> = c.iter().filter(|c| c.field == Field::Series).collect();
        assert_eq!(series.len(), 1);
        assert_eq!(
            (series[0].id, series[0].to.as_str()),
            (2, "Dune Chronicles")
        );
        assert!(c.iter().all(|c| c.id != 4), "a clean book is untouched");
        assert!(plan(&[]).is_empty());
    }

    #[test]
    fn two_different_people_are_not_merged() {
        let books = vec![
            book(1, "A", &["John Smith"], "", 0.0),
            book(2, "B", &["Jon Smith"], "", 0.0),
            book(3, "C", &["J. Smith"], "", 0.0),
        ];
        assert!(plan(&books).is_empty());
    }

    #[test]
    fn a_book_with_two_forms_of_one_author_keeps_one() {
        let books = vec![book(1, "A", &["Frank Herbert", "Herbert, Frank"], "", 0.0)];
        let c = plan(&books);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].to, "Frank Herbert");
    }

    #[test]
    fn the_series_overview_orders_books_and_finds_gaps() {
        let books = vec![
            book(1, "Three", &["A"], "Saga", 3.0),
            book(2, "One", &["A"], "Saga", 1.0),
            book(3, "Five", &["A"], "The Saga Series", 5.0),
            book(4, "Solo", &["A"], "", 0.0),
            book(5, "Half", &["A"], "Other", 1.5),
        ];
        let o = series_overview(&books);
        assert_eq!(o.len(), 2);
        let saga = o.iter().find(|g| g.name.contains("Saga")).unwrap();
        assert_eq!(
            saga.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![2, 1, 3]
        );
        assert_eq!(saga.missing, vec![2, 4]);
        let other = o.iter().find(|g| g.name == "Other").unwrap();
        assert!(other.missing.is_empty());
    }

    #[test]
    fn applying_changes_writes_undoable_edits() {
        let d = std::env::temp_dir().join(format!("bookx-tidy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let md = d.join("b.md");
        std::fs::write(
            &md,
            "---\ntitle: \"T\"\nauthor: \"Herbert, Frank\"\n---\n# One\n\nText.\n",
        )
        .unwrap();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        lib.add_path(&md, false).unwrap();
        let c = plan(&lib.books);
        assert_eq!(c.len(), 1);
        assert_eq!(apply(&mut lib, &c).unwrap(), 1);
        assert_eq!(lib.books[0].meta.authors, vec!["Frank Herbert"]);
        assert_eq!(
            lib.open_book(1).unwrap().metadata().authors,
            vec!["Frank Herbert"]
        );
        assert!(plan(&lib.books).is_empty());
        lib.undo_last().unwrap();
        assert_eq!(lib.books[0].meta.authors, vec!["Herbert, Frank"]);
        let _ = std::fs::remove_dir_all(d);
    }
}
