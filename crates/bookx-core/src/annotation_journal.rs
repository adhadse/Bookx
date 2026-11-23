//! The annotation history: a journal of every highlight, note and bookmark that was added, edited
//! or deleted, with who did it and the complete record as it was before, so that a deletion (by you,
//! by a device sync, by a merge) or an overwrite can be taken back.
//!
//! This module is the GTK-free and database-free part: the types, the diff of two states of a book,
//! restoring and reverting, filtering and grouping for the History view. `bookx-store` keeps the
//! journal in the `annotation_history` table of the catalogue database and writes it **in the same
//! transaction** as the book row (it diffs the old and the new annotations of every book it writes),
//! so no caller can change an annotation without an entry and a crash cannot leave one without the
//! other. The caller says who is responsible through a [`WriteContext`].
//!
//! # Retention
//!
//! Entries younger than [`KEEP_DAYS`] days are kept and so are the newest [`KEEP_MIN`] entries;
//! beyond that, at most [`MAX_ENTRIES`] entries and [`MAX_BYTES`] bytes of stored text.
//!
//! # Restoring
//!
//! [`restore`] puts the record of an entry back ([`Outcome`] says what happened, and why not when
//! it could not: never silently). Restoring is a normal annotation write (source [`Source::Undo`]),
//! so it is journalled too and can itself be undone ([`revert`]).

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::library::{Book, Bookmark, Highlight};

/// Entries younger than this many days are always kept.
pub const KEEP_DAYS: i64 = 90;
/// The newest this many entries are always kept, however old.
pub const KEEP_MIN: usize = 500;
/// Hard cap on the number of entries (the oldest go first).
pub const MAX_ENTRIES: usize = 20_000;
/// Hard cap on the bytes of stored records.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Highlight,
    /// A highlight that carries a note.
    Note,
    Bookmark,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Highlight => "highlight",
            Kind::Note => "note",
            Kind::Bookmark => "bookmark",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Highlight, Kind::Note, Kind::Bookmark]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Added,
    Edited,
    Deleted,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Added => "added",
            Action::Edited => "edited",
            Action::Deleted => "deleted",
        }
    }

    pub fn parse(s: &str) -> Option<Action> {
        [Action::Added, Action::Edited, Action::Deleted]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

/// Who changed it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Source {
    #[default]
    You,
    /// An annotation sync with this device (`Kobo Libra`).
    Device(String),
    Import,
    Merge,
    /// Restored from the history, or the library's own Undo.
    Undo,
}

impl Source {
    /// The value stored in the database: `you`, `device:<name>`, `import`, `merge`, `undo`.
    pub fn as_db(&self) -> String {
        match self {
            Source::You => "you".into(),
            Source::Device(d) => format!("device:{d}"),
            Source::Import => "import".into(),
            Source::Merge => "merge".into(),
            Source::Undo => "undo".into(),
        }
    }

    pub fn from_db(s: &str) -> Source {
        match s {
            "import" => Source::Import,
            "merge" => Source::Merge,
            "undo" => Source::Undo,
            _ => match s.strip_prefix("device:") {
                Some(d) => Source::Device(d.to_string()),
                None => Source::You,
            },
        }
    }
}

/// Who is responsible for the books a save writes. `default` for every book without its own entry.
#[derive(Debug, Clone, Default)]
pub struct WriteContext {
    pub default: Source,
    pub per_book: HashMap<u64, Source>,
}

impl WriteContext {
    pub fn by(source: Source) -> WriteContext {
        WriteContext {
            default: source,
            per_book: HashMap::new(),
        }
    }

    pub fn source_for(&self, book: u64) -> &Source {
        self.per_book.get(&book).unwrap_or(&self.default)
    }
}

/// A highlight (with its note) or a bookmark, complete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Highlight(Highlight),
    Bookmark(Bookmark),
}

impl Record {
    pub fn id(&self) -> &str {
        match self {
            Record::Highlight(h) => &h.id,
            Record::Bookmark(b) => &b.id,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Record::Highlight(h) if h.note.trim().is_empty() => Kind::Highlight,
            Record::Highlight(_) => Kind::Note,
            Record::Bookmark(_) => Kind::Bookmark,
        }
    }

    /// The quote, or the bookmark's label.
    pub fn text(&self) -> &str {
        match self {
            Record::Highlight(h) => &h.text,
            Record::Bookmark(b) => &b.label,
        }
    }

    pub fn note(&self) -> &str {
        match self {
            Record::Highlight(h) => &h.note,
            Record::Bookmark(_) => "",
        }
    }

    pub fn spine(&self) -> usize {
        match self {
            Record::Highlight(h) => h.spine,
            Record::Bookmark(b) => b.spine,
        }
    }

    /// The text to show in a list: the quote, whitespace squeezed, cut at `max` characters.
    pub fn excerpt(&self, max: usize) -> String {
        let t: String = self.text().split_whitespace().collect::<Vec<_>>().join(" ");
        if t.chars().count() <= max {
            t
        } else {
            let mut s: String = t.chars().take(max.saturating_sub(1)).collect();
            s.push('…');
            s
        }
    }
}

/// One change.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Unique and rising (the database row id).
    pub id: u64,
    pub at: DateTime<Utc>,
    /// Changes written together (one save, one bulk sync) share a group.
    pub group: u64,
    pub book: u64,
    /// The book's title and authors at the time, so the entry survives the book.
    pub title: String,
    pub authors: String,
    pub kind: Kind,
    pub action: Action,
    pub source: Source,
    /// The record as it was before the change (for [`Action::Added`]: the record that was added).
    pub record: Record,
}

// -- recording ----------------------------------------------------------------

/// The annotations of one book, as the journal compares them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snap {
    pub highlights: Vec<Highlight>,
    pub bookmarks: Vec<Bookmark>,
}

impl Snap {
    pub fn of(book: &Book) -> Snap {
        Snap {
            highlights: book.highlights.clone(),
            bookmarks: book.bookmarks.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.highlights.is_empty() && self.bookmarks.is_empty()
    }
}

/// A highlight that only moved (its place was re-checked against the text) is not a change the
/// reader made.
fn only_moved(a: &Highlight, b: &Highlight) -> bool {
    let mut x = a.clone();
    x.spine = b.spine;
    x.start = b.start;
    x.end = b.end;
    x == *b
}

/// What changed from `old` to `new`, matched by id: added records, edited ones (the record as it
/// was) and deleted ones. Order: deletions, edits, additions, each in list order.
pub fn diff(old: &Snap, new: &Snap) -> Vec<(Action, Record)> {
    let mut out = Vec::new();
    let nh: HashMap<&str, &Highlight> = new.highlights.iter().map(|h| (h.id.as_str(), h)).collect();
    let oh: HashMap<&str, &Highlight> = old.highlights.iter().map(|h| (h.id.as_str(), h)).collect();
    let nb: HashMap<&str, &Bookmark> = new.bookmarks.iter().map(|h| (h.id.as_str(), h)).collect();
    let ob: HashMap<&str, &Bookmark> = old.bookmarks.iter().map(|h| (h.id.as_str(), h)).collect();
    for h in &old.highlights {
        if !nh.contains_key(h.id.as_str()) {
            out.push((Action::Deleted, Record::Highlight(h.clone())));
        }
    }
    for b in &old.bookmarks {
        if !nb.contains_key(b.id.as_str()) {
            out.push((Action::Deleted, Record::Bookmark(b.clone())));
        }
    }
    for h in &old.highlights {
        if let Some(n) = nh.get(h.id.as_str()) {
            if *n != h && !only_moved(h, n) {
                out.push((Action::Edited, Record::Highlight(h.clone())));
            }
        }
    }
    for b in &old.bookmarks {
        if let Some(n) = nb.get(b.id.as_str()) {
            if *n != b {
                out.push((Action::Edited, Record::Bookmark(b.clone())));
            }
        }
    }
    for h in &new.highlights {
        if !oh.contains_key(h.id.as_str()) {
            out.push((Action::Added, Record::Highlight(h.clone())));
        }
    }
    for b in &new.bookmarks {
        if !ob.contains_key(b.id.as_str()) {
            out.push((Action::Added, Record::Bookmark(b.clone())));
        }
    }
    out
}

// -- restoring ----------------------------------------------------------------

/// What restoring one entry did (or why it did not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Put back (it was gone).
    Restored,
    /// The current version was replaced by the earlier one.
    Replaced,
    /// A reverted addition: the record was taken out again.
    Removed,
    /// The record is already in the book exactly like that.
    AlreadyThere,
    /// A different record marks the same passage (or the same bookmark place) already.
    SamePassage,
    /// The book is not in the library any more.
    BookMissing,
    /// Additions are not restored (nothing was lost).
    NotApplicable,
}

impl Outcome {
    /// The book changed.
    pub fn changed(self) -> bool {
        matches!(
            self,
            Outcome::Restored | Outcome::Replaced | Outcome::Removed
        )
    }
}

fn same_place(a: &Record, b: &Record) -> bool {
    match (a, b) {
        (Record::Highlight(x), Record::Highlight(y)) => {
            x.spine == y.spine && x.start == y.start && x.end == y.end && x.text == y.text
        }
        (Record::Bookmark(x), Record::Bookmark(y)) => {
            x.spine == y.spine && (x.frac - y.frac).abs() < 1e-9 && x.label == y.label
        }
        _ => false,
    }
}

fn current(book: &Book, r: &Record) -> Option<Record> {
    match r {
        Record::Highlight(h) => book
            .highlights
            .iter()
            .find(|x| x.id == h.id)
            .map(|x| Record::Highlight(x.clone())),
        Record::Bookmark(b) => book
            .bookmarks
            .iter()
            .find(|x| x.id == b.id)
            .map(|x| Record::Bookmark(x.clone())),
    }
}

fn all_records(book: &Book) -> Vec<Record> {
    book.highlights
        .iter()
        .cloned()
        .map(Record::Highlight)
        .chain(book.bookmarks.iter().cloned().map(Record::Bookmark))
        .collect()
}

/// What restoring `entry` would do to `book` (`None` = not in the library), without doing it.
pub fn check(entry: &Entry, book: Option<&Book>) -> Outcome {
    let Some(book) = book else {
        return Outcome::BookMissing;
    };
    match entry.action {
        Action::Added => Outcome::NotApplicable,
        Action::Deleted => match current(book, &entry.record) {
            Some(_) => Outcome::AlreadyThere,
            None if all_records(book)
                .iter()
                .any(|r| same_place(r, &entry.record)) =>
            {
                Outcome::SamePassage
            }
            None => Outcome::Restored,
        },
        Action::Edited => match current(book, &entry.record) {
            Some(c) if c == entry.record => Outcome::AlreadyThere,
            Some(_) => Outcome::Replaced,
            None => Outcome::Restored,
        },
    }
}

fn put(book: &mut Book, r: &Record) {
    match r {
        Record::Highlight(h) => match book.highlights.iter_mut().find(|x| x.id == h.id) {
            Some(x) => *x = h.clone(),
            None => book.highlights.push(h.clone()),
        },
        Record::Bookmark(b) => match book.bookmarks.iter_mut().find(|x| x.id == b.id) {
            Some(x) => *x = b.clone(),
            None => book.bookmarks.push(b.clone()),
        },
    }
}

/// Puts the record of `entry` back into `book`.
pub fn restore(entry: &Entry, book: Option<&mut Book>) -> Outcome {
    let out = check(entry, book.as_deref());
    if let (Some(book), true) = (book, matches!(out, Outcome::Restored | Outcome::Replaced)) {
        put(book, &entry.record);
    }
    out
}

/// Takes `entry` back, whatever it was: an addition is removed, a deletion or edit is restored.
pub fn revert(entry: &Entry, book: Option<&mut Book>) -> Outcome {
    if entry.action != Action::Added {
        return restore(entry, book);
    }
    let Some(book) = book else {
        return Outcome::BookMissing;
    };
    let before = (book.highlights.len(), book.bookmarks.len());
    match &entry.record {
        Record::Highlight(h) => book.highlights.retain(|x| x.id != h.id),
        Record::Bookmark(b) => book.bookmarks.retain(|x| x.id != b.id),
    }
    if before == (book.highlights.len(), book.bookmarks.len()) {
        Outcome::AlreadyThere
    } else {
        Outcome::Removed
    }
}

// -- listing --------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionFilter {
    #[default]
    All,
    Deleted,
    Edited,
}

/// What the History view shows.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub action: ActionFilter,
    pub source: Option<Source>,
    pub book: Option<u64>,
    /// Matched (case-insensitively) in the quote, the note, the label, the title and the authors.
    pub query: String,
}

impl Filter {
    pub fn matches(&self, e: &Entry) -> bool {
        let action = match self.action {
            ActionFilter::All => true,
            ActionFilter::Deleted => e.action == Action::Deleted,
            ActionFilter::Edited => e.action == Action::Edited,
        };
        if !action {
            return false;
        }
        if self.source.as_ref().is_some_and(|a| *a != e.source) {
            return false;
        }
        if self.book.is_some_and(|b| b != e.book) {
            return false;
        }
        let q = self.query.trim().to_lowercase();
        q.is_empty()
            || [e.record.text(), e.record.note(), &e.title, &e.authors]
                .iter()
                .any(|s| s.to_lowercase().contains(&q))
    }
}

/// Entries that belong together: one source did one thing to one book in one save.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub group: u64,
    pub book: u64,
    pub title: String,
    pub authors: String,
    pub source: Source,
    pub action: Action,
    /// The newest entry's time.
    pub at: DateTime<Utc>,
    /// Newest first.
    pub entries: Vec<Entry>,
}

impl Group {
    /// How many entries of each kind: (highlights, notes, bookmarks).
    pub fn counts(&self) -> (usize, usize, usize) {
        let n = |k| self.entries.iter().filter(|e| e.kind == k).count();
        (n(Kind::Highlight), n(Kind::Note), n(Kind::Bookmark))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub groups: Vec<Group>,
}

/// The filtered entries as groups per day, newest first. `offset` is the viewer's time zone.
pub fn group_by_day(entries: &[Entry], filter: &Filter, offset: FixedOffset) -> Vec<Day> {
    let mut groups: Vec<Group> = Vec::new();
    for e in entries.iter().rev().filter(|e| filter.matches(e)) {
        match groups.iter_mut().rev().take(8).find(|g| {
            g.group == e.group && g.book == e.book && g.action == e.action && g.source == e.source
        }) {
            Some(g) => g.entries.push(e.clone()),
            None => groups.push(Group {
                group: e.group,
                book: e.book,
                title: e.title.clone(),
                authors: e.authors.clone(),
                source: e.source.clone(),
                action: e.action,
                at: e.at,
                entries: vec![e.clone()],
            }),
        }
    }
    let mut days: Vec<Day> = Vec::new();
    for g in groups {
        let date = g.at.with_timezone(&offset).date_naive();
        match days.last_mut() {
            Some(d) if d.date == date => d.groups.push(g),
            _ => days.push(Day {
                date,
                groups: vec![g],
            }),
        }
    }
    days
}

/// The devices that appear in the journal (for the filter).
pub fn devices(entries: &[Entry]) -> Vec<String> {
    let set: BTreeSet<&str> = entries
        .iter()
        .filter_map(|e| match &e.source {
            Source::Device(d) => Some(d.as_str()),
            _ => None,
        })
        .collect();
    set.into_iter().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    pub(crate) fn hl(id: &str, start: i32, text: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 1,
            start,
            end: start + text.len() as i32,
            color: "yellow".into(),
            style: "background".into(),
            note: String::new(),
            text: text.into(),
            created: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
        }
    }

    pub(crate) fn bm(id: &str, frac: f64) -> Bookmark {
        Bookmark {
            id: id.into(),
            spine: 2,
            frac,
            label: format!("Page {id}"),
            created: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
        }
    }

    fn book(highlights: Vec<Highlight>, bookmarks: Vec<Bookmark>) -> Book {
        let mut b = test_book(7);
        b.highlights = highlights;
        b.bookmarks = bookmarks;
        b
    }

    pub(crate) fn test_book(id: u64) -> Book {
        let now = Utc::now();
        Book {
            id,
            file: format!("{id}.epub"),
            cover: None,
            meta: crate::opf::Metadata {
                title: format!("Book {id}"),
                authors: vec!["Ann Author".into()],
                ..Default::default()
            },
            added: now,
            modified: now,
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
            source_ids: Vec::new(),
        }
    }

    fn entry(n: u64, at: DateTime<Utc>, action: Action, r: Record) -> Entry {
        Entry {
            id: n,
            at,
            group: n,
            book: 7,
            title: "Book 7".into(),
            authors: "Ann Author".into(),
            kind: r.kind(),
            action,
            source: Source::You,
            record: r,
        }
    }

    fn snap(h: Vec<Highlight>, b: Vec<Bookmark>) -> Snap {
        Snap {
            highlights: h,
            bookmarks: b,
        }
    }

    #[test]
    fn diff_finds_deleted_edited_and_added() {
        let old = snap(
            vec![hl("a", 0, "one"), hl("b", 10, "two")],
            vec![bm("x", 0.1)],
        );
        let mut b2 = hl("b", 10, "two");
        b2.note = "mine".into();
        let new = snap(vec![b2, hl("c", 20, "three")], vec![]);
        let d = diff(&old, &new);
        let acts: Vec<(Action, &str)> = d.iter().map(|(a, r)| (*a, r.id())).collect();
        assert_eq!(
            acts,
            vec![
                (Action::Deleted, "a"),
                (Action::Deleted, "x"),
                (Action::Edited, "b"),
                (Action::Added, "c"),
            ]
        );
        // the edit keeps the record as it was: no note
        assert_eq!(d[2].1.note(), "");
    }

    #[test]
    fn a_moved_highlight_is_not_a_change() {
        let old = snap(vec![hl("a", 0, "one")], vec![]);
        let mut moved = hl("a", 5, "one");
        moved.spine = 3;
        assert!(diff(&old, &snap(vec![moved], vec![])).is_empty());
        assert!(diff(&old, &old.clone()).is_empty());
    }

    #[test]
    fn kind_follows_the_note() {
        let mut h = hl("a", 0, "one");
        assert_eq!(Record::Highlight(h.clone()).kind(), Kind::Highlight);
        h.note = "n".into();
        assert_eq!(Record::Highlight(h).kind(), Kind::Note);
        assert_eq!(Record::Bookmark(bm("x", 0.5)).kind(), Kind::Bookmark);
    }

    #[test]
    fn restore_of_a_deletion_gives_the_record_back_exactly() {
        let mut h = hl("a", 4, "the quick  fox");
        h.note = "n\nm".into();
        h.color = "#aabbcc".into();
        h.style = "underline".into();
        let old = snap(vec![h.clone()], vec![bm("x", 0.25)]);
        let mut b = book(vec![], vec![]);
        for (a, r) in diff(&old, &Snap::default()) {
            let e = entry(1, Utc::now(), a, r);
            assert_eq!(restore(&e, Some(&mut b)), Outcome::Restored);
        }
        assert_eq!(b.highlights, vec![h]);
        assert_eq!(b.bookmarks, vec![bm("x", 0.25)]);
    }

    #[test]
    fn restore_is_idempotent_and_explains_conflicts() {
        let h = hl("a", 4, "fox");
        let e = entry(1, Utc::now(), Action::Deleted, Record::Highlight(h.clone()));
        let mut b = book(vec![], vec![]);
        assert_eq!(restore(&e, Some(&mut b)), Outcome::Restored);
        assert_eq!(restore(&e, Some(&mut b)), Outcome::AlreadyThere);
        assert_eq!(b.highlights.len(), 1);
        // the same passage under another id (a device sent it again)
        let mut c = book(vec![hl("zz", 4, "fox")], vec![]);
        assert_eq!(restore(&e, Some(&mut c)), Outcome::SamePassage);
        assert_eq!(c.highlights.len(), 1);
        // no book
        assert_eq!(restore(&e, None), Outcome::BookMissing);
        // additions are nothing to restore
        let add = entry(2, Utc::now(), Action::Added, Record::Highlight(h));
        assert_eq!(restore(&add, Some(&mut c)), Outcome::NotApplicable);
        let m = entry(
            3,
            Utc::now(),
            Action::Deleted,
            Record::Bookmark(bm("x", 0.3)),
        );
        let mut d = book(vec![], vec![bm("other", 0.3)]);
        d.bookmarks[0].label = "Page x".into();
        assert_eq!(restore(&m, Some(&mut d)), Outcome::SamePassage);
    }

    #[test]
    fn restoring_an_edit_brings_back_the_earlier_version() {
        let before = hl("a", 4, "fox");
        let mut now = before.clone();
        now.note = "device note".into();
        let e = entry(
            1,
            Utc::now(),
            Action::Edited,
            Record::Highlight(before.clone()),
        );
        let mut b = book(vec![now], vec![]);
        assert_eq!(check(&e, Some(&b)), Outcome::Replaced);
        assert_eq!(restore(&e, Some(&mut b)), Outcome::Replaced);
        assert_eq!(b.highlights, vec![before]);
        assert_eq!(restore(&e, Some(&mut b)), Outcome::AlreadyThere);
    }

    #[test]
    fn revert_undoes_a_restore() {
        let h = hl("a", 4, "fox");
        let added = entry(1, Utc::now(), Action::Added, Record::Highlight(h.clone()));
        let mut b = book(vec![h], vec![]);
        assert_eq!(revert(&added, Some(&mut b)), Outcome::Removed);
        assert!(b.highlights.is_empty());
        assert_eq!(revert(&added, Some(&mut b)), Outcome::AlreadyThere);
        assert_eq!(revert(&added, None), Outcome::BookMissing);
    }

    /// A small deterministic generator, so the property tests need no extra crate.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    fn random_snap(r: &mut Lcg) -> Snap {
        let mut s = Snap::default();
        for i in 0..r.next(8) {
            let mut h = hl(
                &format!("h{}", r.next(6)),
                (i * 10) as i32,
                &format!("t{}", r.next(50)),
            );
            if r.next(2) == 0 {
                h.note = format!("note {}", r.next(9));
            }
            if !s.highlights.iter().any(|x: &Highlight| x.id == h.id) {
                s.highlights.push(h);
            }
        }
        for _ in 0..r.next(4) {
            let b = bm(&format!("b{}", r.next(4)), r.next(100) as f64 / 100.0);
            if !s.bookmarks.iter().any(|x: &Bookmark| x.id == b.id) {
                s.bookmarks.push(b);
            }
        }
        s
    }

    #[test]
    fn property_restoring_every_entry_of_a_change_undoes_it() {
        let mut r = Lcg(42);
        for _ in 0..300 {
            let old = random_snap(&mut r);
            let new = random_snap(&mut r);
            let mut b = book(new.highlights.clone(), new.bookmarks.clone());
            // edits and additions first: a deleted record's place may be taken by an edited twin
            let mut changes = diff(&old, &new);
            changes.sort_by_key(|(a, _)| *a == Action::Deleted);
            for (a, rec) in changes {
                let e = entry(1, Utc::now(), a, rec);
                let first = revert(&e, Some(&mut b));
                // idempotent: a second revert changes nothing
                let again = revert(&e, Some(&mut b));
                assert!(!again.changed(), "{first:?} then {again:?}");
            }
            // every record of the old state is back, byte for byte (SamePassage keeps a twin)
            for h in &old.highlights {
                let found = b.highlights.iter().find(|x| x.id == h.id);
                let twin = b.highlights.iter().any(|x| {
                    same_place(&Record::Highlight(x.clone()), &Record::Highlight(h.clone()))
                });
                assert!(
                    found.is_some_and(|f| f == h || only_moved(h, f)) || (found.is_none() && twin),
                    "{h:?}"
                );
            }
            for m in &old.bookmarks {
                let found = b.bookmarks.iter().find(|x| x.id == m.id);
                let twin = b.bookmarks.iter().any(|x| {
                    same_place(&Record::Bookmark(x.clone()), &Record::Bookmark(m.clone()))
                });
                assert!(found == Some(m) || (found.is_none() && twin), "{m:?}");
            }
        }
    }

    #[test]
    fn groups_per_day_book_source_and_save() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let d1 = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();
        let d2 = Utc.with_ymd_and_hms(2026, 5, 2, 10, 0, 0).unwrap();
        let mut es = Vec::new();
        for i in 0..3 {
            let mut e = entry(
                10 + i,
                d1,
                Action::Deleted,
                Record::Highlight(hl(&format!("h{i}"), 0, "quote")),
            );
            e.group = 10;
            e.source = Source::Device("Kobo Libra".into());
            es.push(e);
        }
        es.push(entry(
            20,
            d2,
            Action::Edited,
            Record::Highlight(hl("z", 0, "other")),
        ));
        let days = group_by_day(&es, &Filter::default(), tz);
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date, d2.date_naive());
        assert_eq!(days[1].groups.len(), 1);
        assert_eq!(days[1].groups[0].entries.len(), 3);
        assert_eq!(days[1].groups[0].counts(), (3, 0, 0));
        let f = Filter {
            action: ActionFilter::Deleted,
            ..Filter::default()
        };
        assert_eq!(group_by_day(&es, &f, tz).len(), 1);
        let f = Filter {
            source: Some(Source::You),
            ..Filter::default()
        };
        assert_eq!(group_by_day(&es, &f, tz)[0].groups.len(), 1);
        let f = Filter {
            query: "OTHER".into(),
            ..Filter::default()
        };
        assert_eq!(group_by_day(&es, &f, tz)[0].groups[0].entries[0].id, 20);
        let f = Filter {
            book: Some(99),
            ..Filter::default()
        };
        assert!(group_by_day(&es, &f, tz).is_empty());
        assert_eq!(devices(&es), vec!["Kobo Libra".to_string()]);
    }

    #[test]
    fn a_time_zone_moves_the_day() {
        let t = Utc.with_ymd_and_hms(2026, 5, 1, 23, 30, 0).unwrap();
        let es = vec![entry(
            1,
            t,
            Action::Deleted,
            Record::Highlight(hl("a", 0, "x")),
        )];
        let east = FixedOffset::east_opt(3600).unwrap();
        let d = group_by_day(&es, &Filter::default(), east);
        assert_eq!(d[0].date, NaiveDate::from_ymd_opt(2026, 5, 2).unwrap());
    }

    #[test]
    fn excerpt_squeezes_and_cuts() {
        let r = Record::Highlight(hl("a", 0, "one  two\n three four five"));
        assert_eq!(r.excerpt(100), "one two three four five");
        assert_eq!(r.excerpt(8), "one two…");
    }
}
