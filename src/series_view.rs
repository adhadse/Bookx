//! The *Series* view of the library (roadmap PP-G-3): every series with its books in reading order,
//! the numbers that are missing, how much of it was read, and a preview of where a book lands when
//! its series number is typed in any style (`1`, `01`, `I`, `Book Two`).
//!
//! No GTK in here, so it is unit tested; the window is `ui/organize.rs`.

use crate::i18n::ntr;
use crate::i18n::trf;
use bookx_sources::tidy::{self, SeriesGroup};

use crate::core::library::Book;
use crate::core::opf::format_index;

/// A book in a series with how far the reader is.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub index: f64,
    pub title: String,
    pub status: &'static str,
}

/// One series of the library.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub name: String,
    pub entries: Vec<Entry>,
    pub read: usize,
    pub reading: usize,
    pub unread: usize,
    pub missing: Vec<u32>,
    /// Things worth a look: two books with one number, books without a number.
    pub problems: Vec<String>,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    /// `2 of 5 read`
    pub fn progress_text(&self) -> String {
        let mut s = format!("{} of {} read", self.read, self.total());
        if self.reading > 0 {
            s += &format!(", {} in progress", self.reading);
        }
        s
    }

    /// `missing #3, #4`; long runs are shortened to `#3–#9`. Empty when nothing is missing.
    pub fn missing_text(&self) -> String {
        missing_text(&self.missing)
    }
}

pub fn missing_text(missing: &[u32]) -> String {
    if missing.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < missing.len() {
        let mut j = i;
        while j + 1 < missing.len() && missing[j + 1] == missing[j] + 1 {
            j += 1;
        }
        if j - i >= 2 {
            parts.push(format!("#{}–{}", missing[i], missing[j]));
        } else {
            parts.extend(missing[i..=j].iter().map(|n| format!("#{n}")));
        }
        i = j + 1;
    }
    format!("missing {}", parts.join(", "))
}

fn problems(g: &SeriesGroup) -> Vec<String> {
    let mut out = Vec::new();
    let mut numbered: Vec<f64> = g
        .entries
        .iter()
        .map(|e| e.index)
        .filter(|i| *i > 0.0)
        .collect();
    numbered.sort_by(|a, b| a.total_cmp(b));
    let mut dup: Vec<String> = Vec::new();
    for w in numbered.windows(2) {
        if (w[0] - w[1]).abs() < 1e-9 {
            let s = format_index(w[0]);
            if !dup.contains(&s) {
                dup.push(s);
            }
        }
    }
    if !dup.is_empty() {
        out.push(format!("more than one book is #{}", dup.join(", #")));
    }
    let none = g.entries.iter().filter(|e| e.index <= 0.0).count();
    if none > 0 {
        out.push(trf(
            "{} without a number",
            &[&ntr("{} book", "{} books", none)],
        ));
    }
    out
}

/// All series of the library, sorted by name.
pub fn summaries(books: &[Book]) -> Vec<Summary> {
    tidy::series_overview(books)
        .into_iter()
        .map(|g| {
            let entries: Vec<Entry> = g
                .entries
                .iter()
                .map(|e| Entry {
                    id: e.id,
                    index: e.index,
                    title: e.title.clone(),
                    status: books
                        .iter()
                        .find(|b| b.id == e.id)
                        .map_or("unread", |b| b.status()),
                })
                .collect();
            let count = |s: &str| entries.iter().filter(|e| e.status == s).count();
            Summary {
                name: g.name.clone(),
                read: count("finished"),
                reading: count("reading"),
                unread: count("unread"),
                missing: g.missing.clone(),
                problems: problems(&g),
                entries,
            }
        })
        .collect()
}

/// What the number preview needs to know about the other books of the library.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesBook {
    pub series: String,
    pub index: f64,
    pub title: String,
}

/// The books that belong to a series, except `except` (the book being edited).
pub fn context(books: &[Book], except: Option<u64>) -> Vec<SeriesBook> {
    books
        .iter()
        .filter(|b| Some(b.id) != except && !b.meta.series.trim().is_empty())
        .map(|b| SeriesBook {
            series: b.meta.series.clone(),
            index: b.meta.series_index,
            title: b.meta.title.clone(),
        })
        .collect()
}

/// What typing `written` as the series number means, and where the book would sort.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberPreview {
    pub value: Option<f64>,
    pub text: String,
}

/// `Book Two` → 2. `books` are the other books of the library (see [`context`]).
pub fn number_preview(series: &str, written: &str, books: &[SeriesBook]) -> NumberPreview {
    let written = written.trim();
    if written.is_empty() {
        return NumberPreview {
            value: None,
            text: String::new(),
        };
    }
    let Some(v) = tidy::parse_series_index(written) else {
        return NumberPreview {
            value: None,
            text: trf("“{}” is not a number I can read", &[&written]),
        };
    };
    let mut text = format!("= {}", format_index(v));
    if series.trim().is_empty() {
        return NumberPreview {
            value: Some(v),
            text,
        };
    }
    let key = tidy::series_key(series);
    let mut others: Vec<(f64, &str)> = books
        .iter()
        .filter(|b| tidy::series_key(&b.series) == key)
        .map(|b| (b.index, b.title.as_str()))
        .collect();
    others.sort_by(|a, b| a.0.total_cmp(&b.0));
    if others.iter().any(|(i, _)| (*i - v).abs() < 1e-9) {
        let t = others
            .iter()
            .find(|(i, _)| (*i - v).abs() < 1e-9)
            .unwrap()
            .1;
        text += &format!(" · same number as “{t}”");
    } else {
        let before = others.iter().rev().find(|(i, _)| *i < v);
        let after = others.iter().find(|(i, _)| *i > v);
        match (before, after) {
            (Some((_, b)), Some((_, a))) => text += &format!(" · sorts between “{b}” and “{a}”"),
            (Some((_, b)), None) => text += &format!(" · sorts after “{b}”"),
            (None, Some((_, a))) => text += &format!(" · sorts before “{a}”"),
            (None, None) => text += " · the only book of this series so far",
        }
    }
    NumberPreview {
        value: Some(v),
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::opf::Metadata;

    fn book(id: u64, title: &str, series: &str, idx: f64, progress: f64) -> Book {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":["A"],"author_sort":"A","publisher":"","language":"","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        b.id = id;
        b.progress = progress;
        b.meta = Metadata {
            title: title.into(),
            series: series.into(),
            series_index: idx,
            ..b.meta
        };
        b
    }

    #[test]
    fn a_series_lists_order_gaps_and_progress() {
        let books = vec![
            book(1, "Dune", "Dune Chronicles", 1.0, 1.0),
            book(2, "Children of Dune", "Dune Chronicles", 3.0, 0.3),
            book(3, "God Emperor", "Dune Chronicles", 4.0, 0.0),
            book(4, "Standalone", "", 0.0, 0.0),
        ];
        let s = summaries(&books);
        assert_eq!(s.len(), 1);
        let d = &s[0];
        assert_eq!(
            d.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!((d.read, d.reading, d.unread), (1, 1, 1));
        assert_eq!(d.progress_text(), "1 of 3 read, 1 in progress");
        assert_eq!(d.missing_text(), "missing #2");
        assert!(d.problems.is_empty());
    }

    #[test]
    fn long_runs_of_missing_numbers_are_shortened() {
        assert_eq!(missing_text(&[]), "");
        assert_eq!(missing_text(&[3, 4]), "missing #3, #4");
        assert_eq!(missing_text(&[2, 3, 4, 5, 9]), "missing #2–5, #9");
    }

    #[test]
    fn duplicate_and_missing_numbers_are_problems() {
        let books = vec![
            book(1, "A", "S", 2.0, 0.0),
            book(2, "B", "S", 2.0, 0.0),
            book(3, "C", "S", 0.0, 0.0),
        ];
        let s = summaries(&books);
        assert_eq!(
            s[0].problems,
            vec!["more than one book is #2", "1 book without a number"]
        );
    }

    #[test]
    fn numbers_are_read_in_every_style_and_the_sort_position_is_shown() {
        let all = vec![
            book(1, "Dune", "Dune Chronicles", 1.0, 0.0),
            book(2, "God Emperor", "Dune Chronicles", 4.0, 0.0),
            book(3, "Other", "", 0.0, 0.0),
        ];
        let books = context(&all, None);
        assert_eq!(books.len(), 2, "books without a series are left out");
        assert_eq!(
            context(&all, Some(1)).len(),
            1,
            "the edited book is left out"
        );
        for (w, v) in [
            ("1", 1.0),
            ("01", 1.0),
            ("II", 2.0),
            ("Book Two", 2.0),
            ("#3", 3.0),
        ] {
            assert_eq!(
                number_preview("Dune Chronicles", w, &books).value,
                Some(v),
                "{w}"
            );
        }
        let p = number_preview("Dune Chronicles", "Book Two", &books);
        assert_eq!(p.text, "= 2 · sorts between “Dune” and “God Emperor”");
        let p = number_preview("The Dune Chronicles", "5", &books);
        assert_eq!(p.text, "= 5 · sorts after “God Emperor”");
        let p = number_preview("Dune Chronicles", "I", &books);
        assert!(p.text.contains("same number as “Dune”"), "{}", p.text);
        let p = number_preview("Dune Chronicles", "I", &context(&all, Some(1)));
        assert!(p.text.contains("sorts before “God Emperor”"), "{}", p.text);
        assert_eq!(
            number_preview("New", "3", &books).text,
            "= 3 · the only book of this series so far"
        );
        assert_eq!(number_preview("", "3", &books).text, "= 3");
        let bad = number_preview("S", "soon", &books);
        assert_eq!(bad.value, None);
        assert!(bad.text.contains("not a number"));
        assert_eq!(number_preview("S", "  ", &books).text, "");
    }
}
