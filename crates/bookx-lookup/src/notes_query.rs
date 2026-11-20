//! The query behind the *Notes & Highlights* window: every highlight of every book as a flat row,
//! full-text search, filters (colour, has a note, book, dates), grouping (book, month, colour,
//! book tag) and the label for where a passage is. No GTK, no library types: the application turns
//! its books into [`NoteRow`]s.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};

use crate::export::{AnnotationItem, HighlightStyle};

/// One highlight with what is needed to show, find and export it.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteRow {
    pub book_id: u64,
    pub highlight_id: String,
    pub book_title: String,
    pub book_author: Option<String>,
    pub book_tags: Vec<String>,
    pub spine: usize,
    pub chapter: Option<String>,
    pub text: String,
    pub note: String,
    pub color: String,
    pub style: HighlightStyle,
    pub created: DateTime<Utc>,
    /// Set for a PDF highlight: the (1-based) page.
    pub page: Option<usize>,
    /// The text could not be found in the book: the note has no place on the page.
    pub unplaced: bool,
}

impl NoteRow {
    /// Where the passage is: `page 12`, else the chapter title, else `Chapter 3`.
    pub fn place(&self) -> String {
        match (self.page, self.chapter.as_deref().filter(|c| !c.is_empty())) {
            (Some(p), _) => format!("page {p}"),
            (None, Some(c)) => c.to_string(),
            _ => format!("Chapter {}", self.spine + 1),
        }
    }

    pub fn to_item(&self) -> AnnotationItem {
        AnnotationItem {
            book_title: self.book_title.clone(),
            book_author: self.book_author.clone(),
            chapter: if self.page.is_some() {
                None
            } else {
                self.chapter.clone()
            },
            text: self.text.clone(),
            note: Some(self.note.clone()).filter(|n| !n.trim().is_empty()),
            style: self.style,
            color: Some(self.color.clone()).filter(|c| !c.is_empty()),
            created: self.created,
            location: self.page.map(|p| format!("page {p}")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    /// Words that must all occur in the quote, note, title, author or chapter (any case).
    pub text: String,
    /// Keep only these colours (empty: all).
    pub colors: Vec<String>,
    /// `Some(true)`: only highlights with a note; `Some(false)`: only without.
    pub has_note: Option<bool>,
    pub book: Option<u64>,
    /// First day to include.
    pub from: Option<NaiveDate>,
    /// Last day to include.
    pub to: Option<NaiveDate>,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        *self == Filter::default()
    }

    pub fn matches(&self, r: &NoteRow) -> bool {
        if let Some(b) = self.book {
            if r.book_id != b {
                return false;
            }
        }
        if !self.colors.is_empty() && !self.colors.iter().any(|c| c.eq_ignore_ascii_case(&r.color))
        {
            return false;
        }
        if let Some(want) = self.has_note {
            if want == r.note.trim().is_empty() {
                return false;
            }
        }
        let day = r.created.date_naive();
        if self.from.is_some_and(|f| day < f) || self.to.is_some_and(|t| day > t) {
            return false;
        }
        let words: Vec<String> = self
            .text
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();
        if !words.is_empty() {
            let hay = format!(
                "{}\n{}\n{}\n{}\n{}",
                r.text,
                r.note,
                r.book_title,
                r.book_author.as_deref().unwrap_or(""),
                r.chapter.as_deref().unwrap_or("")
            )
            .to_lowercase();
            if !words.iter().all(|w| hay.contains(w.as_str())) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Group {
    #[default]
    Book,
    Month,
    Color,
    Tag,
    None,
}

impl Group {
    pub fn parse(s: &str) -> Group {
        match s {
            "month" | "date" => Group::Month,
            "color" | "colour" => Group::Color,
            "tag" => Group::Tag,
            "none" => Group::None,
            _ => Group::Book,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Newest first.
    #[default]
    Newest,
    Oldest,
    /// Book order, then position in the book.
    InBook,
}

/// A titled run of rows (indices into the slice that was queried).
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<usize>,
}

/// Indices of the rows that pass `f`, in the order `sort` asks for.
pub fn query(rows: &[NoteRow], f: &Filter, sort: Sort) -> Vec<usize> {
    let mut v: Vec<usize> = (0..rows.len()).filter(|&i| f.matches(&rows[i])).collect();
    match sort {
        Sort::Newest => v.sort_by(|&a, &b| rows[b].created.cmp(&rows[a].created)),
        Sort::Oldest => v.sort_by(|&a, &b| rows[a].created.cmp(&rows[b].created)),
        Sort::InBook => v.sort_by(|&a, &b| {
            let (x, y) = (&rows[a], &rows[b]);
            x.book_title
                .to_lowercase()
                .cmp(&y.book_title.to_lowercase())
                .then(x.book_id.cmp(&y.book_id))
                .then(x.spine.cmp(&y.spine))
                .then(x.page.cmp(&y.page))
                .then(x.created.cmp(&y.created))
        }),
    }
    v
}

/// `November 2026`.
pub fn month_title(d: DateTime<Utc>) -> String {
    const M: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    format!("{} {}", M[d.month0() as usize], d.year())
}

/// Splits the already ordered `order` into sections; sections appear in order of first row. A row
/// with several book tags is listed under each of them; one without goes under "No tag".
pub fn group(rows: &[NoteRow], order: &[usize], by: Group) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut put = |title: String, i: usize| match out.iter_mut().find(|s| s.title == title) {
        Some(s) => s.rows.push(i),
        None => out.push(Section {
            title,
            rows: vec![i],
        }),
    };
    for &i in order {
        let r = &rows[i];
        match by {
            Group::None => put("All highlights".into(), i),
            Group::Book => put(
                match r.book_author.as_deref().filter(|a| !a.trim().is_empty()) {
                    Some(a) => format!("{} · {}", r.book_title, a),
                    None => r.book_title.clone(),
                },
                i,
            ),
            Group::Month => put(month_title(r.created), i),
            Group::Color => put(
                if r.color.is_empty() {
                    "No colour".into()
                } else {
                    let mut c = r.color.clone();
                    c[..1].make_ascii_uppercase();
                    c
                },
                i,
            ),
            Group::Tag => {
                if r.book_tags.is_empty() {
                    put("No tag".into(), i);
                }
                for t in &r.book_tags {
                    put(t.clone(), i);
                }
            }
        }
    }
    out
}

/// `2026-11-13` or `13.11.2026`-free: only ISO dates; empty is `None`.
pub fn parse_day(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

/// The colours present, in first-seen order, for the colour filter.
pub fn colors_of(rows: &[NoteRow]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for r in rows {
        if !r.color.is_empty() && !v.contains(&r.color) {
            v.push(r.color.clone());
        }
    }
    v
}

/// Midnight UTC of a day, for tests and callers that need a time.
pub fn day_start(d: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(book: u64, title: &str, text: &str, note: &str, color: &str, day: u32) -> NoteRow {
        NoteRow {
            book_id: book,
            highlight_id: format!("h{book}-{day}"),
            book_title: title.into(),
            book_author: Some("A. Author".into()),
            book_tags: vec![],
            spine: day as usize,
            chapter: Some(format!("Ch {day}")),
            text: text.into(),
            note: note.into(),
            color: color.into(),
            style: HighlightStyle::Highlight,
            created: Utc.with_ymd_and_hms(2026, 11, day, 10, 0, 0).unwrap(),
            page: None,
            unplaced: false,
        }
    }

    fn rows() -> Vec<NoteRow> {
        let mut pdf = row(2, "Beta", "Pdf quote", "", "green", 3);
        pdf.page = Some(12);
        vec![
            row(1, "Alpha", "Fear is the mind-killer", "litany", "yellow", 1),
            row(1, "Alpha", "A beginning is the time", "", "green", 5),
            pdf,
            row(2, "Beta", "Another QUOTE", "mine", "yellow", 9),
        ]
    }

    #[test]
    fn search_needs_every_word_in_quote_note_title_or_chapter_in_any_case() {
        let r = rows();
        let q = |t: &str| {
            query(
                &r,
                &Filter {
                    text: t.into(),
                    ..Default::default()
                },
                Sort::InBook,
            )
        };
        assert_eq!(q("fear MIND"), vec![0]);
        assert_eq!(q("litany"), vec![0], "the note is searched");
        assert_eq!(q("beta another"), vec![3], "title and quote together");
        assert_eq!(q("ch 5"), vec![1], "the chapter is searched");
        assert!(q("nothing like it").is_empty());
        assert_eq!(q("").len(), 4);
    }

    #[test]
    fn filters_combine_colour_note_book_and_days() {
        let r = rows();
        let f = |f: Filter| query(&r, &f, Sort::Oldest);
        assert_eq!(
            f(Filter {
                colors: vec!["Yellow".into()],
                ..Default::default()
            }),
            vec![0, 3]
        );
        assert_eq!(
            f(Filter {
                has_note: Some(true),
                book: Some(1),
                ..Default::default()
            }),
            vec![0]
        );
        assert_eq!(
            f(Filter {
                has_note: Some(false),
                ..Default::default()
            }),
            vec![2, 1]
        );
        assert_eq!(
            f(Filter {
                from: parse_day("2026-11-03"),
                to: parse_day("2026-11-05"),
                ..Default::default()
            }),
            vec![2, 1],
            "both ends are included"
        );
        assert!(Filter::default().is_empty());
    }

    #[test]
    fn newest_first_and_book_order_sort() {
        let r = rows();
        assert_eq!(
            query(&r, &Filter::default(), Sort::Newest),
            vec![3, 1, 2, 0]
        );
        assert_eq!(
            query(&r, &Filter::default(), Sort::InBook),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn groups_by_book_month_colour_and_tag() {
        let mut r = rows();
        r[0].book_tags = vec!["sf".into(), "classic".into()];
        r[1].book_tags = vec!["sf".into()];
        let all = query(&r, &Filter::default(), Sort::InBook);
        let g = group(&r, &all, Group::Book);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].title, "Alpha · A. Author");
        assert_eq!(g[1].rows, vec![2, 3]);
        let g = group(&r, &all, Group::Color);
        assert_eq!(g[0].title, "Yellow");
        assert_eq!(g[0].rows, vec![0, 3]);
        let g = group(&r, &all, Group::Month);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].title, "November 2026");
        let g = group(&r, &all, Group::Tag);
        let titles: Vec<_> = g.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["sf", "classic", "No tag"]);
        assert_eq!(g[0].rows, vec![0, 1]);
        assert_eq!(group(&r, &all, Group::None).len(), 1);
        assert_eq!(Group::parse("colour"), Group::Color);
    }

    #[test]
    fn pdf_highlights_show_their_page_and_export_as_items() {
        let r = rows();
        assert_eq!(r[2].place(), "page 12");
        assert_eq!(r[0].place(), "Ch 1");
        let mut bare = r[0].clone();
        bare.chapter = None;
        assert_eq!(bare.place(), "Chapter 2", "no title: the chapter number");
        let it = r[2].to_item();
        assert_eq!(it.location.as_deref(), Some("page 12"));
        assert_eq!(it.chapter, None);
        assert_eq!(it.note, None);
        assert_eq!(r[0].to_item().note.as_deref(), Some("litany"));
    }

    #[test]
    fn colours_present_are_listed_once_in_order() {
        assert_eq!(colors_of(&rows()), vec!["yellow", "green"]);
        assert_eq!(parse_day("nope"), None);
        assert_eq!(day_start(parse_day("2026-11-03").unwrap()).day(), 3);
    }
}
