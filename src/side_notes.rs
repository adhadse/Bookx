//! What the reader's *Notes & Highlights* page lists: the highlights of the open book, filtered by
//! a search text, colour, style and "has a note", grouped by chapter. No GTK.

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub spine: usize,
    pub start: i32,
    pub chapter: String,
    pub text: String,
    pub note: String,
    pub color: String,
    pub style: String,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub text: String,
    pub colors: Vec<String>,
    pub styles: Vec<String>,
    pub has_note: bool,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        *self == Filter::default()
    }

    pub fn matches(&self, e: &Entry) -> bool {
        if !self.colors.is_empty() && !self.colors.iter().any(|c| c.eq_ignore_ascii_case(&e.color))
        {
            return false;
        }
        if !self.styles.is_empty() && !self.styles.iter().any(|s| s.eq_ignore_ascii_case(&e.style))
        {
            return false;
        }
        if self.has_note && e.note.trim().is_empty() {
            return false;
        }
        let hay = format!("{}\n{}\n{}", e.text, e.note, e.chapter).to_lowercase();
        self.text
            .split_whitespace()
            .all(|w| hay.contains(&w.to_lowercase()))
    }
}

/// A line of the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// A chapter heading with the number of entries under it.
    Chapter(String, usize),
    /// An index into the entries that were queried.
    Entry(usize),
}

/// The rows to show: the entries that pass the filter in book order, under a heading per chapter.
pub fn rows(entries: &[Entry], f: &Filter) -> Vec<Row> {
    let mut idx: Vec<usize> = (0..entries.len())
        .filter(|&i| f.matches(&entries[i]))
        .collect();
    idx.sort_by_key(|&i| (entries[i].spine, entries[i].start, entries[i].created));
    let mut out = Vec::new();
    let mut i = 0;
    while i < idx.len() {
        let spine = entries[idx[i]].spine;
        let n = idx[i..]
            .iter()
            .take_while(|&&j| entries[j].spine == spine)
            .count();
        out.push(Row::Chapter(entries[idx[i]].chapter.clone(), n));
        out.extend(idx[i..i + n].iter().map(|&j| Row::Entry(j)));
        i += n;
    }
    out
}

/// How many entries a list of rows holds.
pub fn count(rows: &[Row]) -> usize {
    rows.iter().filter(|r| matches!(r, Row::Entry(_))).count()
}

/// Cuts a quote for the list: at most `max` characters, at a word, with an ellipsis.
pub fn shorten(text: &str, max: usize) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        return t;
    }
    let cut: String = t.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end())
}

/// A short date for the entry line: `13 Nov 2026`.
pub fn date_text(d: DateTime<Utc>) -> String {
    d.with_timezone(&chrono::Local)
        .format("%d %b %Y")
        .to_string()
}

/// The entries from the library's highlights (PDF ones are not part of the EPUB list).
pub fn colour_label(name: &str) -> String {
    use crate::i18n::{tr, N_};
    let known = match name {
        "yellow" => N_("Yellow"),
        "purple" => N_("Purple"),
        "orange" => N_("Orange"),
        "red" => N_("Red"),
        "blue" => N_("Blue"),
        "green" => N_("Green"),
        _ => "",
    };
    if !known.is_empty() {
        return tr(known);
    }
    let mut c = name.to_string();
    if let Some(f) = c.get_mut(..1) {
        f.make_ascii_uppercase();
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn e(
        id: &str,
        spine: usize,
        start: i32,
        color: &str,
        style: &str,
        note: &str,
        text: &str,
    ) -> Entry {
        Entry {
            id: id.into(),
            spine,
            start,
            chapter: format!("Chapter {}", spine + 1),
            text: text.into(),
            note: note.into(),
            color: color.into(),
            style: style.into(),
            created: Utc.with_ymd_and_hms(2026, 11, 1, 10, 0, 0).unwrap(),
        }
    }

    fn sample() -> Vec<Entry> {
        vec![
            e("c", 2, 5, "blue", "underline", "", "third chapter quote"),
            e(
                "a2",
                0,
                90,
                "yellow",
                "background",
                "remember this",
                "later in one",
            ),
            e("a1", 0, 10, "yellow", "background", "", "early in one"),
            e(
                "b",
                1,
                0,
                "red",
                "squiggly",
                "a thought",
                "the second chapter",
            ),
        ]
    }

    fn ids(es: &[Entry], rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Chapter(t, n) => format!("#{t}:{n}"),
                Row::Entry(i) => es[*i].id.clone(),
            })
            .collect()
    }

    #[test]
    fn groups_by_chapter_in_book_order() {
        let es = sample();
        let r = rows(&es, &Filter::default());
        assert_eq!(
            ids(&es, &r),
            [
                "#Chapter 1:2",
                "a1",
                "a2",
                "#Chapter 2:1",
                "b",
                "#Chapter 3:1",
                "c"
            ]
        );
        assert_eq!(count(&r), 4);
    }

    #[test]
    fn filters_by_colour_style_note_and_text() {
        let es = sample();
        let f = |f: Filter| ids(&es, &rows(&es, &f));
        assert_eq!(
            f(Filter {
                colors: vec!["Red".into()],
                ..Default::default()
            }),
            ["#Chapter 2:1", "b"]
        );
        assert_eq!(
            f(Filter {
                styles: vec!["underline".into(), "squiggly".into()],
                ..Default::default()
            }),
            ["#Chapter 2:1", "b", "#Chapter 3:1", "c"]
        );
        assert_eq!(
            f(Filter {
                has_note: true,
                ..Default::default()
            }),
            ["#Chapter 1:1", "a2", "#Chapter 2:1", "b"]
        );
        assert_eq!(
            f(Filter {
                text: "THOUGHT second".into(),
                ..Default::default()
            }),
            ["#Chapter 2:1", "b"],
            "the search looks in the note too, all words must match"
        );
        assert_eq!(
            f(Filter {
                text: "chapter 3".into(),
                ..Default::default()
            }),
            ["#Chapter 3:1", "c"],
            "and in the chapter name"
        );
        assert!(f(Filter {
            text: "zzz".into(),
            ..Default::default()
        })
        .is_empty());
    }

    #[test]
    fn filters_combine() {
        let es = sample();
        let r = rows(
            &es,
            &Filter {
                colors: vec!["yellow".into()],
                has_note: true,
                text: "later".into(),
                ..Default::default()
            },
        );
        assert_eq!(ids(&es, &r), ["#Chapter 1:1", "a2"]);
        assert!(Filter::default().is_empty());
    }

    #[test]
    fn a_big_list_is_filtered_fast() {
        // typing in the search box rebuilds the rows at every key: 5000 highlights must not be noticeable
        // (debug builds get plenty of room)
        let es: Vec<Entry> = (0..5000)
            .map(|i| {
                e(
                    &format!("h{i}"),
                    i % 200,
                    (i / 200) as i32,
                    ["yellow", "red", "blue"][i % 3],
                    "background",
                    if i % 4 == 0 { "a note" } else { "" },
                    &format!("highlighted passage number {i} of a big book"),
                )
            })
            .collect();
        let t = std::time::Instant::now();
        for q in ["h", "hi", "high", "passage number 4", "zzz"] {
            let f = Filter {
                text: q.into(),
                has_note: true,
                ..Default::default()
            };
            let r = rows(&es, &f);
            assert!(count(&r) <= 1250);
        }
        let ms = t.elapsed().as_millis() as f64
            * std::env::var("BOOKX_PERF_SCALE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0);
        assert!(ms < 1500.0, "five searches took {ms} ms");
    }

    #[test]
    fn long_quotes_are_cut_at_a_word() {
        assert_eq!(shorten("a  b\n c", 10), "a b c");
        assert_eq!(shorten("alpha beta gamma delta", 12), "alpha beta…");
        assert_eq!(shorten("abcdefghijklmnop", 5), "abcde…");
        assert_eq!(colour_label("yellow"), "Yellow");
        assert_eq!(colour_label(""), "");
    }

    #[test]
    fn every_highlight_colour_has_a_label() {
        for (name, _) in crate::ui::reader::HIGHLIGHT_COLORS {
            let l = colour_label(name);
            assert!(l.chars().next().unwrap().is_uppercase(), "{name}");
            assert_eq!(l.to_lowercase(), name);
        }
    }
}
