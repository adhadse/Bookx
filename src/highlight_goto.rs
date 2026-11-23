//! Where a highlight is, as a place the reader can go to. No GTK in here, so it is unit tested.
//!
//! A highlight made in Bookx is stored as a chapter (`spine`) and a character range of the reader's
//! text of that chapter. One that came from a device (KOReader, a Kobo) is stored the same way, but
//! it was placed by *searching for its text*, and the device's own position was thrown away. This
//! module turns any stored highlight into one verified `(spine, start, end)`:
//!
//! 1. the stored range, when it still holds the quote (and, for a synced highlight, when it is also
//!    the occurrence next to where the device had it);
//! 2. the KOReader xpointer (`/body/DocFragment[12]/body/div/p[3]/text().123`): `DocFragment[N]` is
//!    the N-th spine item, the rest of the path is walked in the chapter's markup and gives how far
//!    into the chapter the quote is, and of several occurrences the nearest one wins;
//! 3. the chapter title the device sent, looked up in the contents;
//! 4. the quote anywhere in the book (closest chapter to what is known first).
//!
//! What cannot be found is [`Outcome::Missing`] and carries the chapter to open instead.

use serde::{Deserialize, Serialize};

use crate::core::anchor::{self, BookChapters, Chapters};
use crate::core::library::{Highlight, Library};

/// What a device told about where a highlight is. All of it is a hint: only the text is trusted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hint {
    pub chapter: String,
    pub pos0: String,
    pub pos1: String,
    pub page: Option<i64>,
}

impl Hint {
    pub fn is_empty(&self) -> bool {
        self.chapter.trim().is_empty()
            && self.pos0.trim().is_empty()
            && self.pos1.trim().is_empty()
            && self.page.is_none()
    }
}

/// A KOReader (crengine) xpointer taken apart.
#[derive(Debug, Clone, PartialEq)]
pub struct XPointer {
    /// The `N` of `DocFragment[N]` (1 when there is no index).
    pub fragment: usize,
    /// The elements after the fragment: `body`, `div`, `p[3]` as `(name, 1-based index)`.
    pub path: Vec<(String, usize)>,
    /// The characters into the element's text the pointer is at (`text().123`, `.123`).
    pub offset: Option<usize>,
}

/// Parses `/body/DocFragment[12]/body/div/p[3]/text().123`. `None` for anything else (a PDF position,
/// an empty string, a path that has no fragment).
pub fn parse_xpointer(s: &str) -> Option<XPointer> {
    let s = s.trim();
    let rest = s.strip_prefix('/')?;
    let mut parts = rest.split('/');
    if !parts.next()?.eq_ignore_ascii_case("body") {
        return None;
    }
    let frag = parts.next()?;
    let (name, idx) = split_index(frag);
    if !name.eq_ignore_ascii_case("DocFragment") {
        return None;
    }
    let fragment = idx.unwrap_or(1).max(1);
    let mut path = Vec::new();
    let mut offset = None;
    let tail: Vec<&str> = parts.collect();
    for (n, seg) in tail.iter().enumerate() {
        let mut seg = *seg;
        if n + 1 == tail.len() {
            if let Some((a, o)) = seg.rsplit_once('.') {
                if !o.is_empty() && o.bytes().all(|b| b.is_ascii_digit()) {
                    offset = o.parse().ok();
                    seg = a;
                }
            }
        }
        if seg.is_empty() {
            continue;
        }
        let (name, idx) = split_index(seg);
        if name == "text()" {
            continue;
        }
        path.push((name.to_ascii_lowercase(), idx.unwrap_or(1).max(1)));
    }
    Some(XPointer {
        fragment,
        path,
        offset,
    })
}

fn split_index(seg: &str) -> (&str, Option<usize>) {
    match seg.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']').parse().ok()),
        None => (seg, None),
    }
}

/// How many characters of text there are, as a reader counts them (runs of white space are one).
fn text_len(s: &str) -> usize {
    let mut n = 0;
    let mut space = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !space {
                n += 1;
            }
            space = true;
        } else {
            n += 1;
            space = false;
        }
    }
    n
}

fn skipped(n: roxmltree::Node) -> bool {
    n.ancestors().any(|a| {
        a.is_element()
            && matches!(
                a.tag_name().name().to_ascii_lowercase().as_str(),
                "head" | "style" | "script"
            )
    })
}

/// How far into the chapter's text (0..1) the xpointer is, from the chapter's markup. `None` when
/// the markup cannot be read or has no text.
pub fn fraction_in_markup(xhtml: &str, x: &XPointer) -> Option<f64> {
    let doc = roxmltree::Document::parse_with_options(
        xhtml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let mut node = doc.root_element();
    for (name, idx) in &x.path {
        let next = node
            .children()
            .filter(|c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(name))
            .nth(idx - 1);
        match next {
            Some(n) => node = n,
            None => break,
        }
    }
    let mut before = 0usize;
    let mut total = 0usize;
    let mut reached = false;
    for n in doc.root().descendants() {
        if n == node {
            reached = true;
        }
        if n.is_text() && !skipped(n) {
            let len = text_len(n.text().unwrap_or(""));
            total += len;
            if !reached {
                before += len;
            }
        }
    }
    if total == 0 {
        return None;
    }
    let inside = x
        .offset
        .map(|o| {
            o.min(
                node.descendants()
                    .filter(|d| d.is_text())
                    .map(|d| text_len(d.text().unwrap_or("")))
                    .sum(),
            )
        })
        .unwrap_or(0);
    Some(((before + inside) as f64 / total as f64).clamp(0.0, 1.0))
}

/// The chapters of a book, and the markup of each (to read an xpointer).
pub trait Source: Chapters {
    fn xhtml(&self, spine: usize) -> Option<String>;
}

impl Source for BookChapters {
    fn xhtml(&self, spine: usize) -> Option<String> {
        let path = self.book().spine_paths().get(spine)?.clone();
        self.book().archive.get_string(&path)
    }
}

/// The text a title is looked up by.
pub fn title_key(s: &str) -> String {
    crate::device_sync::identity(s)
}

/// `(title key, spine)` of the contents entries.
pub fn toc_spines(chapters: &BookChapters) -> Vec<(String, usize)> {
    let entries = chapters.book().toc();
    crate::core::toc::flatten(&entries)
        .iter()
        .filter_map(|t| {
            let path = t.href.split('#').next().unwrap_or(&t.href);
            Some((title_key(&t.title), chapters.index_of(path)?))
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// How a place was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// The stored range holds the quote.
    Stored,
    /// Next to where the device's xpointer points.
    Xpointer,
    /// In the chapter the device named.
    Chapter,
    /// Searched for in the whole book.
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub spine: usize,
    pub start: usize,
    pub end: usize,
    pub how: How,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Found(Target),
    /// The quote is nowhere; `chapter` is the best guess of where it belongs.
    Missing {
        chapter: Option<usize>,
    },
}

/// What the hint says about the place: `(spine, offset into that chapter's text)`.
fn from_hint(src: &dyn Source, hint: &Hint) -> (Option<usize>, Option<usize>) {
    let count = src.count();
    let mut xp_spine = None;
    let mut offset = None;
    if let Some(x) = parse_xpointer(&hint.pos0).or_else(|| parse_xpointer(&hint.pos1)) {
        if count > 0 {
            let s = (x.fragment - 1).min(count - 1);
            xp_spine = Some(s);
            let len = src.texts(s).first().map_or(0, |t| t.len());
            offset = src
                .xhtml(s)
                .and_then(|m| fraction_in_markup(&m, &x))
                .map(|f| (f * len as f64) as usize);
        }
    }
    (xp_spine, offset)
}

/// Finds the place of `h`. `toc` is [`toc_spines`].
pub fn resolve(
    src: &dyn Source,
    h: &Highlight,
    hint: Option<&Hint>,
    toc: &[(String, usize)],
) -> Outcome {
    let quote = h.text.trim();
    let (s0, a0, b0) = anchor::hint_of(h);
    let anchored = !anchor::is_unanchored(h);
    if quote.is_empty() {
        return if anchored && s0 < src.count() {
            Outcome::Found(Target {
                spine: s0,
                start: a0.max(0) as usize,
                end: b0.max(0) as usize,
                how: How::Stored,
            })
        } else {
            Outcome::Missing { chapter: None }
        };
    }
    let hint = hint.filter(|h| !h.is_empty());
    let (xp_spine, xp_offset) = hint.map_or((None, None), |h| from_hint(src, h));
    let chapter_spine = hint
        .map(|h| title_key(&h.chapter))
        .filter(|k| !k.is_empty())
        .and_then(|k| toc.iter().find(|(t, _)| *t == k).map(|(_, s)| *s));
    let stored_ok = anchored
        && src
            .texts(s0)
            .iter()
            .any(|t| anchor::verify(t, a0, b0, quote));
    // the nearest occurrence to the device's xpointer, in its chapter only
    let near_xp = xp_spine.and_then(|s| {
        src.texts(s)
            .iter()
            .find_map(|t| anchor::locate_exact(t, quote, xp_offset))
            .map(|f| Target {
                spine: s,
                start: f.start,
                end: f.end,
                how: How::Xpointer,
            })
    });
    if let Some(t) = near_xp {
        return Outcome::Found(t);
    }
    if stored_ok {
        return Outcome::Found(Target {
            spine: s0,
            start: a0 as usize,
            end: b0 as usize,
            how: How::Stored,
        });
    }
    let (spine_hint, how) = match (xp_spine, chapter_spine) {
        (Some(s), _) => (Some(s), How::Xpointer),
        (None, Some(s)) => (Some(s), How::Chapter),
        (None, None) => (Some(s0).filter(|s| *s < src.count()), How::Search),
    };
    let offset = if xp_spine.is_some() {
        xp_offset
    } else if anchored {
        Some(a0.max(0) as usize)
    } else {
        None
    };
    match anchor::resolve(src, quote, spine_hint, offset) {
        Some(r) => Outcome::Found(Target {
            spine: r.spine,
            start: r.found.start,
            end: r.found.end,
            how: if Some(r.spine) == spine_hint {
                how
            } else {
                How::Search
            },
        }),
        None => Outcome::Missing {
            chapter: xp_spine
                .or(chapter_spine)
                .or_else(|| anchored.then_some(s0)),
        },
    }
}

/// Resolves a highlight of a library book and keeps the place it found on the highlight (its colour,
/// style, text and note are not touched). `None` when the book cannot be opened.
pub fn go_to(
    lib: &mut Library,
    book_id: u64,
    highlight_id: &str,
    hint: Option<&Hint>,
) -> Option<Outcome> {
    let open = lib.open_book(book_id).ok()?;
    let chapters = BookChapters::new(open);
    let toc = toc_spines(&chapters);
    let h = lib
        .get(book_id)?
        .highlights
        .iter()
        .find(|h| h.id == highlight_id)?
        .clone();
    if crate::pdf::is_pdf(&h) {
        return None;
    }
    let out = resolve(&chapters, &h, hint, &toc);
    if let Outcome::Found(t) = out {
        if anchor::is_unanchored(&h)
            || (h.spine, h.start, h.end) != (t.spine, t.start as i32, t.end as i32)
        {
            if let Some(m) = lib
                .get_mut(book_id)
                .and_then(|b| b.highlights.iter_mut().find(|x| x.id == highlight_id))
            {
                anchor::place(m, t.spine, t.start, t.end);
            }
            let _ = lib.save();
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::Arc;

    struct Fake {
        texts: Vec<String>,
        xhtml: Vec<String>,
    }

    impl Chapters for Fake {
        fn count(&self) -> usize {
            self.texts.len()
        }
        fn texts(&self, s: usize) -> Vec<Arc<Vec<char>>> {
            vec![Arc::new(self.texts[s].chars().collect())]
        }
    }

    impl Source for Fake {
        fn xhtml(&self, s: usize) -> Option<String> {
            self.xhtml.get(s).cloned()
        }
    }

    fn page(paras: &[&str]) -> String {
        let body: String = paras.iter().map(|p| format!("<p>{p}</p>")).collect();
        format!(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body><div>{body}</div></body></html>"
        )
    }

    fn fake(chapters: &[&[&str]]) -> Fake {
        Fake {
            texts: chapters.iter().map(|c| c.join("\n")).collect(),
            xhtml: chapters.iter().map(|c| page(c)).collect(),
        }
    }

    fn hl(spine: usize, start: i32, end: i32, text: &str) -> Highlight {
        Highlight {
            id: "h".into(),
            spine,
            start,
            end,
            color: "green".into(),
            style: String::new(),
            note: "n".into(),
            text: text.into(),
            created: Utc::now(),
        }
    }

    const Q: &str = "a compensating transaction";

    #[test]
    fn xpointers_are_taken_apart() {
        let x = parse_xpointer("/body/DocFragment[12]/body/div/p[3]/text().123").unwrap();
        assert_eq!(x.fragment, 12);
        assert_eq!(
            x.path,
            vec![("body".into(), 1), ("div".into(), 1), ("p".into(), 3)]
        );
        assert_eq!(x.offset, Some(123));
        let x = parse_xpointer("/body/DocFragment/body/div[2]/p[7].5").unwrap();
        assert_eq!((x.fragment, x.offset), (1, Some(5)));
        assert_eq!(x.path.last().unwrap(), &("p".to_string(), 7));
        let x = parse_xpointer("/body/DocFragment[3]/body/p/img.0").unwrap();
        assert_eq!(x.path.last().unwrap(), &("img".to_string(), 1));
        assert!(parse_xpointer("").is_none());
        assert!(parse_xpointer("/page/12").is_none());
        assert!(parse_xpointer("/body/div/p").is_none());
        assert!(parse_xpointer("[page=3]").is_none());
    }

    #[test]
    fn the_markup_gives_how_far_into_the_chapter_the_pointer_is() {
        let m = page(&["aaaa", "bbbb", "cccc", "dddd"]);
        let x = parse_xpointer("/body/DocFragment[1]/body/div/p[3]/text().0").unwrap();
        let f = fraction_in_markup(&m, &x).unwrap();
        assert!((f - 0.5).abs() < 1e-9, "{f}");
        let x = parse_xpointer("/body/DocFragment[1]/body/div/p[4]/text().2").unwrap();
        let f = fraction_in_markup(&m, &x).unwrap();
        assert!((f - 14.0 / 16.0).abs() < 1e-9, "{f}");
        // a path that is not in the markup stops at the deepest element that is
        let x = parse_xpointer("/body/DocFragment[1]/body/div/p[9]/text().0").unwrap();
        assert!(fraction_in_markup(&m, &x).is_some());
        assert!(fraction_in_markup("<html><p>broken", &x).is_none());
    }

    #[test]
    fn a_stored_range_that_holds_the_quote_is_kept() {
        let b = fake(&[&["x", "intro"], &[&format!("see {Q} here")]]);
        let start = "see ".len() as i32;
        let h = hl(1, start, start + Q.len() as i32, Q);
        let Outcome::Found(t) = resolve(&b, &h, None, &[]) else {
            panic!()
        };
        assert_eq!((t.spine, t.start, t.how), (1, 4, How::Stored));
    }

    #[test]
    fn a_synced_highlight_without_a_place_is_found_by_its_xpointer_not_the_first_occurrence() {
        // the same words in chapters 0, 2 and 4: DocFragment[3] is spine 2
        let with = |n: &str| format!("before {Q} after {n}");
        let b = fake(&[
            &[&with("zero")],
            &["other"],
            &["filler one", &with("two")],
            &["other"],
            &[&with("four")],
        ]);
        let (s, a, e) = anchor::unplaced(0);
        let h = hl(s, a, e, Q);
        let hint = Hint {
            pos0: "/body/DocFragment[3]/body/div/p[2]/text().7".into(),
            ..Default::default()
        };
        let Outcome::Found(t) = resolve(&b, &h, Some(&hint), &[]) else {
            panic!()
        };
        assert_eq!((t.spine, t.how), (2, How::Xpointer));
        assert_eq!(t.start, "filler one\nbefore ".chars().count());
        let hint = Hint {
            pos0: "/body/DocFragment[5]/body/div/p/text().0".into(),
            ..Default::default()
        };
        let Outcome::Found(t) = resolve(&b, &h, Some(&hint), &[]) else {
            panic!()
        };
        assert_eq!(t.spine, 4);
        // without any hint the first one in the book is all there is
        let Outcome::Found(t) = resolve(&b, &h, None, &[]) else {
            panic!()
        };
        assert_eq!((t.spine, t.how), (0, How::Search));
    }

    #[test]
    fn of_several_occurrences_in_one_chapter_the_one_near_the_pointer_wins() {
        let paras: Vec<String> = (0..10)
            .map(|i| {
                if i == 1 || i == 8 {
                    format!("{Q} {i}")
                } else {
                    format!("filler paragraph number {i}")
                }
            })
            .collect();
        let refs: Vec<&str> = paras.iter().map(|s| s.as_str()).collect();
        let b = fake(&[&refs]);
        let (s, a, e) = anchor::unplaced(0);
        let h = hl(s, a, e, Q);
        let near_end = Hint {
            pos0: "/body/DocFragment[1]/body/div/p[9]/text().0".into(),
            ..Default::default()
        };
        let Outcome::Found(t) = resolve(&b, &h, Some(&near_end), &[]) else {
            panic!()
        };
        let before: usize = paras[..8].iter().map(|p| p.chars().count() + 1).sum();
        assert_eq!(t.start, before);
        let near_start = Hint {
            pos0: "/body/DocFragment[1]/body/div/p[2]/text().0".into(),
            ..Default::default()
        };
        let Outcome::Found(t) = resolve(&b, &h, Some(&near_start), &[]) else {
            panic!()
        };
        assert_eq!(t.start, paras[0].chars().count() + 1);
    }

    #[test]
    fn a_stored_place_on_the_wrong_occurrence_is_corrected_by_the_pointer() {
        let with = |n: &str| format!("{Q} in {n}");
        let b = fake(&[&[&with("zero")], &[&with("one")]]);
        let h = hl(0, 0, Q.len() as i32, Q);
        let hint = Hint {
            pos0: "/body/DocFragment[2]/body/div/p/text().0".into(),
            ..Default::default()
        };
        let Outcome::Found(t) = resolve(&b, &h, Some(&hint), &[]) else {
            panic!()
        };
        assert_eq!((t.spine, t.start), (1, 0));
    }

    #[test]
    fn a_fragment_that_is_off_by_some_chapters_still_finds_the_neighbour() {
        let b = fake(&[&["one"], &["two"], &[&format!("x {Q} y")], &["four"]]);
        let (s, a, e) = anchor::unplaced(0);
        let h = hl(s, a, e, Q);
        let hint = Hint {
            pos0: "/body/DocFragment[2]/body/div/p/text().0".into(),
            ..Default::default()
        };
        let Outcome::Found(t) = resolve(&b, &h, Some(&hint), &[]) else {
            panic!()
        };
        assert_eq!(t.spine, 2);
    }

    #[test]
    fn the_chapter_title_decides_between_chapters_when_there_is_no_pointer() {
        let b = fake(&[&[&format!("{Q} a")], &["x"], &[&format!("{Q} b")]]);
        let toc = vec![(title_key("Chapter Two"), 2usize)];
        let (s, a, e) = anchor::unplaced(0);
        let h = hl(s, a, e, Q);
        let hint = Hint {
            chapter: "Chapter two".into(),
            ..Default::default()
        };
        let Outcome::Found(t) = resolve(&b, &h, Some(&hint), &toc) else {
            panic!()
        };
        assert_eq!((t.spine, t.how), (2, How::Chapter));
    }

    #[test]
    fn offsets_are_characters_not_bytes() {
        let q = "Größe über alles — überall";
        let b = fake(&[&[&format!("Ünïcode ✓ prefix {q} suffix")]]);
        let (s, a, e) = anchor::unplaced(0);
        let h = hl(s, a, e, q);
        let Outcome::Found(t) = resolve(&b, &h, None, &[]) else {
            panic!()
        };
        assert_eq!(t.start, "Ünïcode ✓ prefix ".chars().count());
        assert_eq!(t.end - t.start, q.chars().count());
    }

    #[test]
    fn a_quote_that_is_not_in_the_book_is_missing_and_names_the_chapter_to_open() {
        let b = fake(&[&["one"], &["two two two"], &["three"]]);
        let (s, a, e) = anchor::unplaced(0);
        let h = hl(s, a, e, "words that are nowhere in this book");
        let hint = Hint {
            pos0: "/body/DocFragment[2]/body/div/p/text().0".into(),
            ..Default::default()
        };
        assert_eq!(
            resolve(&b, &h, Some(&hint), &[]),
            Outcome::Missing { chapter: Some(1) }
        );
        assert_eq!(
            resolve(&b, &h, None, &[]),
            Outcome::Missing { chapter: None }
        );
    }

    #[test]
    fn a_fragment_past_the_end_is_clamped() {
        let b = fake(&[&["one"], &[&format!("x {Q} y")]]);
        let h = hl(0, -1, -1, Q);
        let hint = Hint {
            pos0: "/body/DocFragment[40]/body/div/p/text().0".into(),
            ..Default::default()
        };
        let Outcome::Found(t) = resolve(&b, &h, Some(&hint), &[]) else {
            panic!()
        };
        assert_eq!(t.spine, 1);
    }

    #[test]
    fn go_to_keeps_the_place_it_found_and_nothing_else() {
        use crate::core::convert::{Chapter, create_epub};
        use crate::core::library::AddOutcome;
        use crate::core::opf::Metadata;
        let dir = std::env::temp_dir().join(format!("bookx-goto-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let md = Metadata {
            title: "T".into(),
            authors: vec!["A".into()],
            language: "en".into(),
            ..Default::default()
        };
        let chapters: Vec<Chapter> = (1..=3)
            .map(|c| Chapter {
                title: format!("Chapter {c}"),
                body: format!(
                    "<h1>Chapter {c}</h1><p>Intro of {c}.</p><p>The saga says {Q} in plain words.</p><p>End of {c}.</p>"
                ),
            })
            .collect();
        let src = dir.join("t.epub");
        std::fs::write(&src, create_epub(&md, &chapters, None).to_bytes().unwrap()).unwrap();
        let mut lib = Library::in_memory(&dir.join("lib")).unwrap();
        let AddOutcome::Added(id) = lib.add_path(&src, true).unwrap() else {
            panic!("not added")
        };
        let (s, a, e) = anchor::unplaced(0);
        lib.get_mut(id).unwrap().highlights.push(hl(s, a, e, Q));
        let hint = Hint {
            pos0: "/body/DocFragment[3]/body/p[3]/text().0".into(),
            ..Default::default()
        };
        let out = go_to(&mut lib, id, "h", Some(&hint)).unwrap();
        let Outcome::Found(t) = out else {
            panic!("{out:?}")
        };
        assert_eq!(t.spine, 2);
        let h = &lib.get(id).unwrap().highlights[0];
        assert_eq!(
            (h.spine, h.start as usize, h.end as usize),
            (2, t.start, t.end)
        );
        assert_eq!(
            (h.color.as_str(), h.note.as_str(), h.text.as_str()),
            ("green", "n", Q)
        );
        // the next time the stored place is enough, with or without the hint
        let Outcome::Found(again) = go_to(&mut lib, id, "h", None).unwrap() else {
            panic!()
        };
        assert_eq!(
            (again.spine, again.start, again.how),
            (2, t.start, How::Stored)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
