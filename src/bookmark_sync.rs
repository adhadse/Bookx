//! Bookmarks that come from a device (a KOReader page bookmark, a Kobo dog-ear), as real Bookx
//! bookmarks. No GTK in here, so it is unit tested.
//!
//! A Bookx bookmark is a chapter (`spine`) and how far into it (`frac`). A device bookmark has no
//! quote to find, only *where the device had it*, so the place is worked out from what the device
//! sent, best first:
//!
//! 1. the KOReader xpointer (`/body/DocFragment[7]/body/p[3]/text().0`): the chapter and, from the
//!    chapter's markup, how far into it (an *exact* place);
//! 2. a Kobo dog-ear: the chapter file and the chapter progress it names (exact);
//! 3. the chapter title the device sent (the start of that chapter), refined by the device's
//!    percentage when that falls inside the chapter;
//! 4. the device's percentage of the whole book;
//! 5. the device's page number as a share of Bookx's own estimate of the book's pages.
//!
//! Anything below 1 and 2 is *approximate* and flagged so (the row says so). A bookmark the book
//! cannot place at all is still kept, at the start of the book and flagged: nothing is lost.
//!
//! **Rules.** A device bookmark is remembered by the key the device gave it in a *ledger*; once
//! remembered it is never imported again, so a bookmark the reader deleted in Bookx stays deleted
//! and one the reader removed on the device is not removed here (nothing is ever deleted by a
//! sync). A bookmark the library already has at the same place is linked to, never added twice,
//! and a bookmark made in Bookx is never changed.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::core::library::{Book, Bookmark};
use crate::highlight_goto::{fraction_in_markup, parse_xpointer, title_key, Hint};

/// Characters of text per page of Bookx's estimate of a book's pages (the reader's own estimate
/// starts from the same density before a chapter is laid out).
pub const PAGE_CHARS: f64 = 2000.0;

/// Two bookmarks of one chapter closer than this are the same place.
pub const SAME_SPOT: f64 = 0.01;

/// Two bookmarks of one chapter closer than this, made on the same day, are the same place.
pub const SAME_DAY_SPOT: f64 = 0.04;

/// What a device says about one bookmark.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceMark {
    /// What names it on the device (stays the same between syncs).
    pub id: String,
    /// The reader's own words; empty when none.
    pub note: String,
    pub chapter: String,
    /// KOReader's xpointer.
    pub pos: String,
    /// The device's own page number.
    pub page: Option<i64>,
    /// 0..=1 in the book.
    pub percentage: Option<f64>,
    /// A Kobo: the chapter file and how far into it.
    pub href: Option<String>,
    pub chapter_progress: Option<f64>,
    /// Seconds since the epoch; 0 when the device does not say.
    pub created: i64,
}

impl DeviceMark {
    /// What to show for it: the reader's words, else `Page 77`, else the chapter.
    pub fn label(&self) -> String {
        let note = self.note.split_whitespace().collect::<Vec<_>>().join(" ");
        if !note.is_empty() {
            return note.chars().take(200).collect();
        }
        if let Some(p) = self.page.filter(|p| *p > 0) {
            return format!("Page {p}");
        }
        let ch = self.chapter.trim();
        if !ch.is_empty() {
            return ch.chars().take(200).collect();
        }
        "Bookmark".to_string()
    }

    /// Where the device had it, as the hint the reader keeps.
    pub fn hint(&self) -> Hint {
        Hint {
            chapter: self.chapter.trim().to_string(),
            pos0: self.pos.trim().to_string(),
            pos1: String::new(),
            page: self.page,
        }
    }
}

/// How a place was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Xpointer,
    ChapterFile,
    Chapter,
    Percentage,
    Page,
    /// Nothing the device said fits the book: the start of it.
    Nowhere,
}

/// A place in the book: the chapter and how far into it (0..1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub spine: usize,
    pub frac: f64,
    pub how: How,
    /// The place is the page the device had, not a guess from a chapter, a percentage or a page
    /// number.
    pub exact: bool,
}

impl Place {
    pub fn unknown() -> Place {
        Place {
            spine: 0,
            frac: 0.0,
            how: How::Nowhere,
            exact: false,
        }
    }
}

/// What the book can tell about itself to read a device's place.
pub trait Geometry {
    fn count(&self) -> usize;
    /// How much text chapter `spine` has (characters).
    fn weight(&self, spine: usize) -> f64;
    fn xhtml(&self, spine: usize) -> Option<String>;
    /// The chapter a contents entry of this title (`highlight_goto::title_key`) leads to.
    fn spine_of_title(&self, key: &str) -> Option<usize>;
    /// The chapter of a file name (`OEBPS/ch1.xhtml`, or the end of it).
    fn spine_of_file(&self, href: &str) -> Option<usize>;
}

fn clamp01(f: f64) -> f64 {
    if f.is_finite() {
        f.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The chapter and the place in it of a fraction of the whole book's text.
fn book_fraction(g: &dyn Geometry, f: f64) -> Option<(usize, f64)> {
    let n = g.count();
    if n == 0 {
        return None;
    }
    let w: Vec<f64> = (0..n).map(|i| g.weight(i).max(0.0)).collect();
    let total: f64 = w.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let target = clamp01(f) * total;
    let mut acc = 0.0;
    for (i, wi) in w.iter().enumerate() {
        if acc + wi >= target || i + 1 == n {
            let frac = if *wi > 0.0 {
                clamp01((target - acc) / wi)
            } else {
                0.0
            };
            return Some((i, frac));
        }
        acc += wi;
    }
    None
}

/// Where in the whole book (0..1) the start of chapter `spine` is, and how much of it the chapter
/// is.
fn chapter_span(g: &dyn Geometry, spine: usize) -> Option<(f64, f64)> {
    let total: f64 = (0..g.count()).map(|i| g.weight(i).max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let before: f64 = (0..spine).map(|i| g.weight(i).max(0.0)).sum();
    Some((before / total, g.weight(spine).max(0.0) / total))
}

/// Works out the place of a device bookmark. Never fails: what cannot be placed is
/// [`Place::unknown`].
pub fn resolve_place(g: &dyn Geometry, m: &DeviceMark) -> Place {
    let count = g.count();
    if count == 0 {
        return Place::unknown();
    }
    // 1. the xpointer
    if let Some(x) = parse_xpointer(&m.pos) {
        let spine = (x.fragment - 1).min(count - 1);
        let f = g.xhtml(spine).and_then(|h| fraction_in_markup(&h, &x));
        return Place {
            spine,
            frac: f.unwrap_or(0.0),
            how: How::Xpointer,
            exact: f.is_some(),
        };
    }
    // 2. a Kobo dog-ear: the chapter file and the progress in it
    if let Some(spine) = m.href.as_deref().and_then(|h| g.spine_of_file(h)) {
        let p = m.chapter_progress.map(clamp01);
        return Place {
            spine,
            frac: p.unwrap_or(0.0),
            how: How::ChapterFile,
            exact: p.is_some(),
        };
    }
    // 3. the chapter the device named
    let by_title = Some(title_key(&m.chapter))
        .filter(|k| !k.is_empty())
        .and_then(|k| g.spine_of_title(&k));
    if let Some(spine) = by_title {
        // the device's percentage refines it when it falls inside that chapter
        let inside = m
            .percentage
            .map(clamp01)
            .zip(chapter_span(g, spine))
            .filter(|(p, (start, len))| *len > 0.0 && *p >= *start && *p <= start + len)
            .map(|(p, (start, len))| clamp01((p - start) / len));
        return Place {
            spine,
            frac: inside.unwrap_or(0.0),
            how: How::Chapter,
            exact: false,
        };
    }
    // 4. the percentage of the book
    if let Some((spine, frac)) = m.percentage.and_then(|p| book_fraction(g, p)) {
        return Place {
            spine,
            frac,
            how: How::Percentage,
            exact: false,
        };
    }
    // 5. the device's page number against Bookx's estimate of the pages
    if let Some(page) = m.page.filter(|p| *p > 0) {
        let total: f64 = (0..count).map(|i| g.weight(i).max(0.0)).sum();
        let pages = (total / PAGE_CHARS).max(1.0);
        if let Some((spine, frac)) = book_fraction(g, (page as f64 - 1.0) / pages) {
            return Place {
                spine,
                frac,
                how: How::Page,
                exact: false,
            };
        }
    }
    Place::unknown()
}

/// What the ledger remembers of a bookmark that came from a device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub book_id: u64,
    /// The library bookmark it is (or was: when it is no longer in the book the reader deleted it
    /// and it stays deleted).
    pub bookmark: String,
    /// `KOReader (Kobo Libra)`, `Kobo`.
    pub source: String,
    /// The place is a guess from a chapter, a percentage or a page number.
    pub approx: bool,
    /// Where the device had it.
    pub hint: Hint,
    /// Added by the sync (false: the library already had a bookmark at that place).
    pub made_here: bool,
}

/// What importing one bookmark did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Added,
    /// The library already had a bookmark there: linked, nothing added.
    Linked,
    /// Handled before (kept, or deleted by the reader since): nothing happens.
    Known,
}

fn when(secs: i64, now: DateTime<Utc>) -> DateTime<Utc> {
    if secs > 0 {
        Utc.timestamp_opt(secs, 0).single().unwrap_or(now)
    } else {
        now
    }
}

/// The bookmark of `book` at this place, if there is one: same chapter and closer than
/// [`SAME_SPOT`], or closer than [`SAME_DAY_SPOT`] and made on the same day.
pub fn bookmark_at(book: &Book, place: &Place, created: DateTime<Utc>) -> Option<usize> {
    book.bookmarks.iter().position(|b| {
        b.spine == place.spine && {
            let d = (b.frac - place.frac).abs();
            d <= SAME_SPOT || (d <= SAME_DAY_SPOT && b.created.date_naive() == created.date_naive())
        }
    })
}

/// Imports one device bookmark into `book`. `key` names it in `entries` (the ledger); an entry
/// that is there already means it was handled: it is never added again, even when the reader
/// deleted it here since. A guessed place is never matched against other bookmarks: two pages of a
/// chapter the device could only name by its title are two bookmarks.
pub fn import(
    book: &mut Book,
    entries: &mut BTreeMap<String, Entry>,
    key: &str,
    m: &DeviceMark,
    place: &Place,
    source: &str,
    now: DateTime<Utc>,
) -> Outcome {
    if entries.contains_key(key) {
        return Outcome::Known;
    }
    let created = when(m.created, now);
    let mut entry = Entry {
        book_id: book.id,
        bookmark: String::new(),
        source: source.to_string(),
        approx: !place.exact,
        hint: m.hint(),
        made_here: false,
    };
    if place.exact {
        if let Some(i) = bookmark_at(book, place, created) {
            entry.bookmark = book.bookmarks[i].id.clone();
            entries.insert(key.to_string(), entry);
            return Outcome::Linked;
        }
    }
    let id = fresh_id(book);
    book.bookmarks.push(Bookmark {
        id: id.clone(),
        spine: place.spine,
        frac: place.frac,
        label: m.label(),
        created,
    });
    entry.bookmark = id;
    entry.made_here = true;
    entries.insert(key.to_string(), entry);
    Outcome::Added
}

fn fresh_id(book: &Book) -> String {
    loop {
        let id = format!("bm-{}", uuid::Uuid::new_v4());
        if !book.bookmarks.iter().any(|b| b.id == id) {
            return id;
        }
    }
}

/// The bookmarks the sync brought: `(book id, bookmark id) -> (where from, approximate)`.
pub fn infos(entries: &BTreeMap<String, Entry>) -> HashMap<(u64, String), (String, bool)> {
    entries
        .values()
        .map(|e| {
            (
                (e.book_id, e.bookmark.clone()),
                (e.source.clone(), e.approx),
            )
        })
        .collect()
}

/// The ledger of the cable sync (the wireless sync keeps its entries in its own ledger).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub entries: BTreeMap<String, Entry>,
}

pub const STORE_FILE: &str = "device-bookmarks.json";

impl Store {
    pub fn path(root: &Path) -> PathBuf {
        root.join(STORE_FILE)
    }

    pub fn load(root: &Path) -> Store {
        std::fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = Self::path(root);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))
    }

    /// The ledger key of a device bookmark of the cable sync.
    pub fn key(book_id: u64, source: &str, device_id: &str) -> String {
        format!("usb/{book_id}/{source}/{device_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A book of `n` chapters with `weights` characters each; chapter `i` has `titles[i]`.
    struct Fake {
        weights: Vec<f64>,
        titles: Vec<&'static str>,
        files: Vec<&'static str>,
    }

    impl Geometry for Fake {
        fn count(&self) -> usize {
            self.weights.len()
        }
        fn weight(&self, s: usize) -> f64 {
            self.weights[s]
        }
        fn xhtml(&self, s: usize) -> Option<String> {
            let body: String = (1..=4).map(|i| format!("<p>para {i} words</p>")).collect();
            (s < self.weights.len()).then(|| {
                format!(
                    "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><div>{body}</div></body></html>"
                )
            })
        }
        fn spine_of_title(&self, key: &str) -> Option<usize> {
            self.titles.iter().position(|t| title_key(t) == key)
        }
        fn spine_of_file(&self, href: &str) -> Option<usize> {
            self.files
                .iter()
                .position(|f| f.ends_with(href) || href.ends_with(f))
        }
    }

    fn fake() -> Fake {
        Fake {
            weights: vec![1000.0, 4000.0, 3000.0, 2000.0],
            titles: vec!["Cover", "One", "Two", "Three"],
            files: vec![
                "OEBPS/c0.xhtml",
                "OEBPS/c1.xhtml",
                "OEBPS/c2.xhtml",
                "OEBPS/c3.xhtml",
            ],
        }
    }

    fn book() -> Book {
        serde_json::from_value(serde_json::json!({
            "id": 1, "file": "a.epub", "meta": {"title": "Dune", "title_sort": "", "authors": ["Frank Herbert"], "author_sort": "", "publisher": "", "language": "", "description": "", "tags": [], "series": "", "series_index": 0.0, "identifiers": [], "published": "", "rights": "", "rating": 0},
            "added": Utc::now(), "modified": Utc::now(), "size": 1
        }))
        .unwrap()
    }

    fn mark(id: &str) -> DeviceMark {
        DeviceMark {
            id: id.into(),
            created: 1_700_000_000,
            ..Default::default()
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0).unwrap()
    }

    #[test]
    fn an_xpointer_gives_the_chapter_and_how_far_into_it() {
        let m = DeviceMark {
            pos: "/body/DocFragment[3]/body/div/p[3]/text().0".into(),
            chapter: "Wrong".into(),
            percentage: Some(0.9),
            ..mark("a")
        };
        let p = resolve_place(&fake(), &m);
        assert_eq!((p.spine, p.how, p.exact), (2, How::Xpointer, true));
        assert!((p.frac - 0.5).abs() < 0.01, "{}", p.frac);
        // a fragment past the end is the last chapter
        let m = DeviceMark {
            pos: "/body/DocFragment[99]/body/div/p[1]/text().0".into(),
            ..mark("a")
        };
        assert_eq!(resolve_place(&fake(), &m).spine, 3);
    }

    #[test]
    fn a_kobo_dog_ear_gives_the_chapter_file_and_its_progress() {
        let m = DeviceMark {
            href: Some("OEBPS/c1.xhtml".into()),
            chapter_progress: Some(0.25),
            ..mark("b")
        };
        let p = resolve_place(&fake(), &m);
        assert_eq!((p.spine, p.how, p.exact), (1, How::ChapterFile, true));
        assert_eq!(p.frac, 0.25);
        // a file the book does not have is not a place
        let m = DeviceMark {
            href: Some("OEBPS/zzz.xhtml".into()),
            ..mark("b")
        };
        assert_eq!(resolve_place(&fake(), &m).how, How::Nowhere);
    }

    #[test]
    fn a_chapter_title_is_the_chapter_and_the_percentage_refines_it() {
        let m = DeviceMark {
            chapter: "TWO".into(),
            ..mark("c")
        };
        let p = resolve_place(&fake(), &m);
        assert_eq!(
            (p.spine, p.frac, p.how, p.exact),
            (2, 0.0, How::Chapter, false)
        );
        // chapter Two spans 0.5..0.8 of the book's text: 0.65 is half way through it
        let m = DeviceMark {
            chapter: "Two".into(),
            percentage: Some(0.65),
            ..mark("c")
        };
        let p = resolve_place(&fake(), &m);
        assert_eq!(p.spine, 2);
        assert!((p.frac - 0.5).abs() < 1e-9, "{}", p.frac);
        // a percentage outside that chapter is not used
        let m = DeviceMark {
            chapter: "Two".into(),
            percentage: Some(0.05),
            ..mark("c")
        };
        assert_eq!(resolve_place(&fake(), &m).frac, 0.0);
    }

    #[test]
    fn a_percentage_and_then_a_page_number_are_guesses() {
        let m = DeviceMark {
            percentage: Some(0.4),
            ..mark("d")
        };
        let p = resolve_place(&fake(), &m);
        assert_eq!((p.spine, p.how, p.exact), (1, How::Percentage, false));
        assert!((p.frac - 0.75).abs() < 1e-9, "{}", p.frac);
        // 10 000 characters are 5 pages of 2000: page 4 starts at 3/5 of the book, a third into
        // the third chapter (0.5..0.8)
        let m = DeviceMark {
            page: Some(4),
            ..mark("d")
        };
        let p = resolve_place(&fake(), &m);
        assert_eq!((p.spine, p.how, p.exact), (2, How::Page, false));
        assert!((p.frac - 1.0 / 3.0).abs() < 1e-9, "{}", p.frac);
    }

    #[test]
    fn what_cannot_be_placed_is_the_start_of_the_book_and_not_exact() {
        let p = resolve_place(&fake(), &mark("e"));
        assert_eq!(p, Place::unknown());
        let empty = Fake {
            weights: vec![],
            titles: vec![],
            files: vec![],
        };
        assert_eq!(resolve_place(&empty, &mark("e")), Place::unknown());
    }

    #[test]
    fn the_label_is_the_note_or_the_page_or_the_chapter() {
        let mut m = mark("a");
        assert_eq!(m.label(), "Bookmark");
        m.chapter = "Litany".into();
        assert_eq!(m.label(), "Litany");
        m.page = Some(77);
        assert_eq!(m.label(), "Page 77");
        m.note = "  where I\nstopped ".into();
        assert_eq!(m.label(), "where I stopped");
        assert_eq!(m.hint().page, Some(77));
    }

    fn exact(spine: usize, frac: f64) -> Place {
        Place {
            spine,
            frac,
            how: How::Xpointer,
            exact: true,
        }
    }

    #[test]
    fn a_new_bookmark_is_added_with_its_place_date_and_origin() {
        let mut b = book();
        let mut e = BTreeMap::new();
        let m = DeviceMark {
            page: Some(77),
            chapter: "Two".into(),
            ..mark("a")
        };
        let o = import(
            &mut b,
            &mut e,
            "k1",
            &m,
            &exact(2, 0.5),
            "KOReader (Kobo Libra)",
            now(),
        );
        assert_eq!(o, Outcome::Added);
        let bm = &b.bookmarks[0];
        assert_eq!((bm.spine, bm.frac, bm.label.as_str()), (2, 0.5, "Page 77"));
        assert_eq!(bm.created.timestamp(), 1_700_000_000);
        let inf = infos(&e);
        assert_eq!(
            inf[&(1, bm.id.clone())],
            ("KOReader (Kobo Libra)".to_string(), false)
        );
        assert!(e["k1"].made_here);
        assert_eq!(e["k1"].hint.chapter, "Two");
    }

    #[test]
    fn a_repeat_adds_nothing_and_a_deleted_bookmark_stays_deleted() {
        let mut b = book();
        let mut e = BTreeMap::new();
        let m = mark("a");
        import(&mut b, &mut e, "k1", &m, &exact(2, 0.5), "KOReader", now());
        assert_eq!(
            import(&mut b, &mut e, "k1", &m, &exact(2, 0.5), "KOReader", now()),
            Outcome::Known
        );
        assert_eq!(b.bookmarks.len(), 1);
        // the reader deletes it in Bookx; the next sync does not bring it back
        b.bookmarks.clear();
        assert_eq!(
            import(&mut b, &mut e, "k1", &m, &exact(2, 0.5), "KOReader", now()),
            Outcome::Known
        );
        assert!(b.bookmarks.is_empty(), "the ledger entry is the tombstone");
        assert!(e.contains_key("k1"));
    }

    #[test]
    fn a_bookmark_the_library_has_at_the_same_place_is_linked_not_added() {
        let mut b = book();
        b.bookmarks.push(Bookmark {
            id: "mine".into(),
            spine: 2,
            frac: 0.505,
            label: "Page 9".into(),
            created: Utc.timestamp_opt(1_000, 0).unwrap(),
        });
        let mut e = BTreeMap::new();
        let o = import(
            &mut b,
            &mut e,
            "k1",
            &mark("a"),
            &exact(2, 0.5),
            "KOReader",
            now(),
        );
        assert_eq!(o, Outcome::Linked);
        assert_eq!(b.bookmarks.len(), 1);
        assert_eq!(
            b.bookmarks[0].label, "Page 9",
            "a bookmark made here is never changed"
        );
        assert_eq!(e["k1"].bookmark, "mine");
        assert!(!e["k1"].made_here);
        // a bookmark nearby on another day is another bookmark, far away in the chapter too
        let o = import(
            &mut b,
            &mut e,
            "k2",
            &mark("b"),
            &exact(2, 0.53),
            "KOReader",
            now(),
        );
        assert_eq!(o, Outcome::Added, "0.03 away and made on another day");
        let o = import(
            &mut b,
            &mut e,
            "k3",
            &mark("c"),
            &exact(1, 0.5),
            "KOReader",
            now(),
        );
        assert_eq!(o, Outcome::Added, "another chapter");
    }

    #[test]
    fn a_close_bookmark_made_on_the_same_day_is_the_same_place() {
        let mut b = book();
        b.bookmarks.push(Bookmark {
            id: "mine".into(),
            spine: 2,
            frac: 0.5,
            label: "Page 9".into(),
            created: Utc.timestamp_opt(1_700_000_100, 0).unwrap(),
        });
        let mut e = BTreeMap::new();
        let o = import(
            &mut b,
            &mut e,
            "k1",
            &mark("a"),
            &exact(2, 0.53),
            "KOReader",
            now(),
        );
        assert_eq!(o, Outcome::Linked);
    }

    #[test]
    fn a_guessed_place_is_kept_flagged_and_never_merged_with_another() {
        let mut b = book();
        let mut e = BTreeMap::new();
        let guess = Place {
            spine: 2,
            frac: 0.0,
            how: How::Chapter,
            exact: false,
        };
        let o = import(&mut b, &mut e, "k1", &mark("a"), &guess, "KOReader", now());
        assert_eq!(o, Outcome::Added);
        let o = import(&mut b, &mut e, "k2", &mark("b"), &guess, "KOReader", now());
        assert_eq!(
            o,
            Outcome::Added,
            "two pages of one chapter named only by title"
        );
        assert_eq!(b.bookmarks.len(), 2);
        let id = b.bookmarks[0].id.clone();
        assert!(infos(&e)[&(1, id)].1, "flagged as approximate");
        // nothing at all known: still kept, at the start, flagged
        let o = import(
            &mut b,
            &mut e,
            "k3",
            &mark("c"),
            &Place::unknown(),
            "Kobo",
            now(),
        );
        assert_eq!(o, Outcome::Added);
        assert_eq!((b.bookmarks[2].spine, b.bookmarks[2].frac), (0, 0.0));
    }

    #[test]
    fn the_cable_ledger_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("bookx-bm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::load(&dir);
        assert!(s.entries.is_empty());
        let k = Store::key(1, "KOReader", "x|y");
        s.entries.insert(
            k.clone(),
            Entry {
                book_id: 1,
                bookmark: "bm-1".into(),
                source: "KOReader".into(),
                ..Default::default()
            },
        );
        s.save(&dir).unwrap();
        assert!(Store::load(&dir).entries.contains_key(&k));
        let _ = std::fs::remove_dir_all(dir);
    }
}
