//! Syncing the reading state between a device and the library: reading progress, highlights and notes.
//!
//! The rule is **progressive and information-conserving**:
//!
//! * progress only ever moves forward (the side that is further along wins, never the other way);
//! * highlights and notes are united by identity (the same id, else the same text), so nothing is
//!   duplicated and nothing is lost; when both sides have the same highlight and only one has a note,
//!   the richer one is kept;
//! * nothing is ever deleted on either side because the other side lacks it.
//!
//! The work is split in three pure steps that the window shows to the user in this order:
//! [`plan`] (what would change), [`Plan::summary`] (the words and counts) and [`apply_to_library`]
//! (the library side; the device side goes through [`NoteSink`]). Reading the device is a thin glue
//! layer at the end of the file ([`read_device`]).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use bookx_serve::reading::{DeviceState, Status};
use bookx_serve::syncplan::{self, Annot, Conflict, Side, SideBook};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::library::{Book, Highlight};
use crate::i18n::{ntr, tr};

/// Settings of the sync, kept in the config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncPrefs {
    /// Sync reading progress and notes from the device whenever it is connected (off by default).
    pub auto_sync: bool,
    /// "Send to <device>" appends the book's notes to the copy it sends.
    pub include_notes: bool,
}

impl Default for SyncPrefs {
    fn default() -> Self {
        SyncPrefs {
            auto_sync: false,
            include_notes: true,
        }
    }
}

/// Title and subtitle of the "add notes" switch (Device window and Preferences).
pub const NOTES_TITLE: &str = crate::i18n::N_("Add notes to the copy sent to the device");
pub const NOTES_SUBTITLE: &str = crate::i18n::N_("The copy on the device gets an extra “My notes” section at the end (readable in any e-reader app) and a KOReader notes file, so KOReader lists them too. Your library copy is not changed.");

/// Title and subtitle of the persistent automatic sync switch in Devices → Settings.
pub const AUTO_SYNC_TITLE: &str = crate::i18n::N_("Always sync when connected");
pub const AUTO_SYNC_SUBTITLE: &str = crate::i18n::N_("For e-readers plugged in with a cable. When this is on, Bookx syncs the reading progress, highlights and notes on every connection without asking. Progress only moves forward, notes are united and nothing is deleted.");

/// The longer explanation behind the "?" button of the automatic sync switch.
pub const AUTO_SYNC_HELP: &str = crate::i18n::N_("This is about a device plugged in with a USB cable (or mounted in Files).\n\nWhen you plug in a Kobo, Kindle or similar reader, Bookx reads the reading progress and the highlights and notes the device keeps: from the Kobo’s own database, or from KOReader’s “.sdr” files if KOReader is installed on it. It merges them with your library. Progress only moves forward, highlights and notes are united, and nothing is deleted on either side.\n\nA Kobo needs no KOReader for this; its own reading state is read. A Kindle’s own state is not readable, only KOReader’s files. If the device has no readable state, nothing happens. Reading progress is never written to the device; notes go to it only as a new KOReader notes file where there is none.\n\nOff by default: the connection banner offers a one-time Sync. Answering it never changes this setting. Turn the switch on to sync on every connection without asking.");

/// The connection banner: one title, one action and the built-in dismiss button.
pub const BANNER_TITLE: &str = crate::i18n::N_("E-reader connected");
pub const BANNER_BUTTON: &str = crate::i18n::N_("Sync");

/// The empty state of the Devices window.
#[allow(dead_code)] // the Devices window track picks these up in its own change
pub const NO_DEVICE_TITLE: &str = crate::i18n::N_("No Device Connected");
#[allow(dead_code)]
pub const NO_DEVICE_DESCRIPTION: &str =
    crate::i18n::N_("Connect your e-reader with USB, or add a WebDAV folder below.");

/// Accessible names of the single grid/list toggle.
#[allow(dead_code)]
pub const GRID_VIEW_NAME: &str = crate::i18n::N_("Grid view");
#[allow(dead_code)]
pub const LIST_VIEW_NAME: &str = crate::i18n::N_("List view");

/// A highlight or note as the device has it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceNote {
    /// The id when the device kept ours (a book sent by Bookx), else `None`.
    pub id: Option<String>,
    pub text: String,
    pub note: String,
    pub created: Option<DateTime<Utc>>,
    /// Colour and style as stored words (`bookx_core::devstyle`); `None` when the device says nothing.
    pub color: Option<String>,
    pub style: Option<String>,
}

/// What a device knows about one of its books.
#[derive(Debug, Clone)]
pub struct DeviceReading {
    pub path: PathBuf,
    pub state: Option<DeviceState>,
    pub notes: Vec<DeviceNote>,
    /// Whether Bookx can put notes next to this device copy (it never touches a notes file that
    /// the device already has).
    pub accepts_notes: bool,
    /// The page bookmarks the device has (KOReader's, a Kobo's dog-ears).
    pub bookmarks: Vec<crate::bookmark_sync::DeviceMark>,
}

/// What the library knows about one of its books.
#[derive(Debug, Clone)]
pub struct LibraryBook {
    pub id: u64,
    pub title: String,
    pub authors: Vec<String>,
    pub progress: f64,
    /// Seconds since the epoch.
    pub last_read: Option<i64>,
    pub highlights: Vec<Highlight>,
    /// What the devices said the look of highlights was the last time (`devstyle::DeviceLooks`).
    pub looks: BTreeMap<String, crate::core::devstyle::Look>,
    /// The device bookmarks an earlier sync handled (`bookmark_sync::Store::key`): they are never
    /// imported again, not even when the reader deleted them here.
    pub bookmark_keys: HashSet<String>,
}

pub fn library_books(books: &[Book]) -> Vec<LibraryBook> {
    books
        .iter()
        .map(|b| LibraryBook {
            id: b.id,
            title: b.meta.title.clone(),
            authors: b.meta.authors.clone(),
            progress: b.progress,
            last_read: b.last_read.map(|d| d.timestamp()),
            // PDF highlights are page rectangles, not EPUB spine positions
            highlights: b
                .highlights
                .iter()
                .filter(|h| !crate::pdf::is_pdf(h))
                .cloned()
                .collect(),
            looks: BTreeMap::new(),
            bookmark_keys: HashSet::new(),
        })
        .collect()
}

/// Adds what the devices said about the look of highlights last time, so a re-read does not undo
/// what the reader changed in Bookx since.
pub fn attach_looks(books: &mut [LibraryBook], root: &Path) {
    let ledger = crate::core::devstyle::DeviceLooks::load(root);
    let marks = crate::bookmark_sync::Store::load(root);
    for b in books {
        b.looks = ledger.of_book(b.id);
        b.bookmark_keys = marks
            .entries
            .iter()
            .filter(|(_, e)| e.book_id == b.id)
            .map(|(k, _)| k.clone())
            .collect();
    }
}

/// Finds where a quote sits in a book, as the position a library highlight is stored with.
pub trait Locator {
    fn locate(&self, book_id: u64, text: &str) -> Option<(usize, i32, i32)>;

    /// Like [`Locator::locate`] when the device named the chapter the quote is in (used to choose
    /// between chapters that hold the same words).
    fn locate_in(
        &self,
        book_id: u64,
        text: &str,
        _chapter: Option<&str>,
    ) -> Option<(usize, i32, i32)> {
        self.locate(book_id, text)
    }

    /// Like [`Locator::locate_in`] when the device also said where it had the quote (xpointer, chapter
    /// title): of several places that hold the words, the one it points at.
    fn locate_hinted(
        &self,
        book_id: u64,
        text: &str,
        chapter: Option<&str>,
        _hint: &crate::highlight_goto::Hint,
    ) -> Option<(usize, i32, i32)> {
        self.locate_in(book_id, text, chapter)
    }

    /// The place of a device bookmark (it has no quote to search for): [`Place::unknown`] when the
    /// book cannot say.
    fn locate_place(
        &self,
        _book_id: u64,
        _mark: &crate::bookmark_sync::DeviceMark,
    ) -> crate::bookmark_sync::Place {
        crate::bookmark_sync::Place::unknown()
    }

    /// Like [`Locator::locate_in`] for a quote that is already highlighted at the places in `taken`
    /// (`(spine, start, end)`): another place that holds the same words, if there is one.
    fn locate_other(
        &self,
        _book_id: u64,
        _text: &str,
        _chapter: Option<&str>,
        _taken: &[(usize, i32, i32)],
    ) -> Option<(usize, i32, i32)> {
        None
    }
}

/// Receives the notes that go to the device.
pub trait NoteSink {
    /// Puts `highlights` of the book titled `title` next to the device copy at `path`; returns how
    /// many were written.
    fn write(
        &mut self,
        path: &Path,
        title: &str,
        highlights: &[Highlight],
    ) -> anyhow::Result<usize>;
}

fn key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// The identity of a quote: its text without case, spacing and punctuation.
pub fn identity(text: &str) -> String {
    key(text)
}

fn same_note(h: &Highlight, n: &DeviceNote) -> bool {
    if let Some(id) = &n.id {
        if *id == h.id {
            return true;
        }
    }
    let (a, b) = (identity(&h.text), identity(&n.text));
    !a.is_empty() && a == b
}

/// A device note that is new to the library, with the place it will be stored at.
#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    pub note: DeviceNote,
    pub spine: usize,
    pub start: i32,
    pub end: i32,
}

/// A device bookmark that is new to the library, with the place it will be stored at.
#[derive(Debug, Clone, PartialEq)]
pub struct BookmarkIn {
    /// The ledger key (`bookmark_sync::Store::key`).
    pub key: String,
    pub mark: crate::bookmark_sync::DeviceMark,
    pub place: crate::bookmark_sync::Place,
    /// `KOReader` or `Kobo`.
    pub source: String,
}

/// Highlights of the library that go to one device copy.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub path: PathBuf,
    pub highlights: Vec<Highlight>,
}

/// Progress the device is ahead with.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressMove {
    pub from: f64,
    pub to: f64,
    pub finished: bool,
    pub source: &'static str,
}

/// What happens to one library book.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BookPlan {
    pub book_id: u64,
    pub title: String,
    pub progress: Option<ProgressMove>,
    /// The library is further along than the device; nothing is done about it.
    pub library_ahead: Option<(f64, f64)>,
    pub highlights_in: Vec<Incoming>,
    /// Device bookmarks the library has not handled yet.
    pub bookmarks_in: Vec<BookmarkIn>,
    /// The colour and style of highlights the library has and the device read says (see
    /// `look_sync`).
    pub looks_in: Vec<crate::look_sync::LookIn>,
    /// `(highlight id, note)`: the device has a note on a highlight the library has without one.
    pub notes_in: Vec<(String, String)>,
    /// Device notes whose text could not be found in the book: they are in `highlights_in` as
    /// notes without a place (`start < 0`, see `bookx_core::anchor`), never at a guessed place.
    pub unplaced: usize,
    pub out: Vec<Outgoing>,
    /// Library highlights the device copy lacks but cannot take (it has a notes file of its own).
    pub held_back: usize,
    /// Highlights whose note differs on the two sides: both stay as they are, nothing is merged
    /// or overwritten (the library keeps its note, the device keeps its own).
    pub conflicts: Vec<Conflict>,
    /// `(library, device)`: the device is ahead but the library copy was read more recently, so
    /// the book was probably started again; progress is left alone.
    pub restarted: Option<(f64, f64)>,
}

impl BookPlan {
    pub fn is_empty(&self) -> bool {
        self.progress.is_none()
            && self.highlights_in.is_empty()
            && self.bookmarks_in.is_empty()
            && !self.looks_in.iter().any(|l| l.set.is_some())
            && self.notes_in.is_empty()
            && self.out.is_empty()
    }

    fn outgoing_ids(&self) -> HashSet<&str> {
        self.out
            .iter()
            .flat_map(|o| o.highlights.iter().map(|h| h.id.as_str()))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    pub books: Vec<BookPlan>,
    /// Device books that are not in the library.
    pub unmatched: usize,
    /// Device books that were matched to a library book.
    pub matched: usize,
}

pub(crate) fn side_of_library(b: &LibraryBook) -> SideBook {
    SideBook {
        id: b.id.to_string(),
        title: b.title.clone(),
        authors: b.authors.clone(),
        progress: b.progress,
        status: if b.progress >= 1.0 {
            Status::Finished
        } else if b.progress > 0.0 {
            Status::Reading
        } else {
            Status::Unread
        },
        last_read: b.last_read,
        // `when` carries the index, so the plan can be mapped back to the highlight
        notes: b
            .highlights
            .iter()
            .enumerate()
            .map(|(i, h)| Annot {
                text: h.text.clone(),
                note: h.note.clone(),
                when: Some(i.to_string()),
            })
            .collect(),
    }
}

fn side_of_device(i: usize, r: &DeviceReading, library: &[LibraryBook]) -> SideBook {
    let stem = r
        .path
        .file_name()
        .map(|n| bookx_serve::devfs::file_stem(&n.to_string_lossy()).to_string())
        .unwrap_or_default();
    let st = r.state.as_ref();
    SideBook {
        id: i.to_string(),
        title: st
            .and_then(|s| s.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or(stem),
        authors: Vec::new(),
        progress: st.map_or(0.0, |s| s.progress),
        status: st.map_or(Status::Unread, |s| s.status),
        last_read: st
            .and_then(|s| s.last_read.as_deref())
            .and_then(bookx_serve::reading::parse_time),
        notes: r
            .notes
            .iter()
            .enumerate()
            .map(|(j, n)| Annot {
                // a note that carries the id of one of our highlights is that highlight
                text: n
                    .id
                    .as_ref()
                    .and_then(|id| {
                        library
                            .iter()
                            .flat_map(|b| b.highlights.iter())
                            .find(|h| h.id == *id)
                    })
                    .map_or_else(|| n.text.clone(), |h| h.text.clone()),
                note: n.note.clone(),
                when: Some(j.to_string()),
            })
            .collect(),
    }
}

fn index_of(a: &Annot) -> Option<usize> {
    a.when.as_deref().and_then(|w| w.parse().ok())
}

/// Decides what a sync would do. Nothing here changes any data. Pairing the books, the progress
/// rule and the highlight/note rules are those of [`bookx_serve::syncplan`]; this function maps
/// its answer onto the library's types (where a highlight is stored, which device copy gets
/// which notes).
pub fn plan(device: &[DeviceReading], library: &[LibraryBook], locator: &dyn Locator) -> Plan {
    let lib_sides: Vec<SideBook> = library.iter().map(side_of_library).collect();
    let dev_sides: Vec<SideBook> = device
        .iter()
        .enumerate()
        .map(|(i, r)| side_of_device(i, r, library))
        .collect();
    let sp = syncplan::plan(&lib_sides, &dev_sides);
    let mut out = Plan {
        unmatched: sp.skipped.len(),
        matched: sp.books.len(),
        ..Default::default()
    };
    // one plan per library book, however many device copies there are
    let mut by_book: BTreeMap<u64, BookPlan> = BTreeMap::new();
    let mut copies: BTreeMap<u64, (usize, usize)> = BTreeMap::new();
    let mut ahead: BTreeMap<u64, f64> = BTreeMap::new();
    let mut seen: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for b in &sp.books {
        let (Ok(bid), Ok(di)) = (b.library_id.parse::<u64>(), b.device_id.parse::<usize>()) else {
            continue;
        };
        let (Some(book), Some(reading)) = (library.iter().find(|l| l.id == bid), device.get(di))
        else {
            continue;
        };
        let bp = by_book.entry(bid).or_insert_with(|| BookPlan {
            book_id: bid,
            title: book.title.clone(),
            ..Default::default()
        });
        let c = copies.entry(bid).or_default();
        if reading.state.is_some() {
            c.0 += 1;
        }
        let source = reading.state.as_ref().map_or("", |s| s.source);
        match b.progress {
            syncplan::Progress::Forward {
                to_side: Side::Library,
                from,
                to,
                finished,
            } => {
                if bp.progress.as_ref().is_none_or(|p| to > p.to) {
                    bp.progress = Some(ProgressMove {
                        from,
                        to,
                        finished: finished || to >= 0.98,
                        source,
                    });
                }
            }
            syncplan::Progress::Forward {
                to_side: Side::Device,
                from,
                ..
            } => {
                c.1 += 1;
                let e = ahead.entry(bid).or_insert(from);
                *e = e.max(from);
            }
            syncplan::Progress::InSync => {}
        }
        if b.behind_but_newer.is_some() {
            let dev_p = dev_sides[di].effective_progress();
            bp.restarted = Some((book.progress, dev_p));
        }
        // device -> library
        for a in &b.notes_to_library {
            let Some(n) = index_of(a).and_then(|j| reading.notes.get(j)) else {
                continue;
            };
            if identity(&n.text).is_empty() && n.id.is_none() {
                continue;
            }
            let known = seen.entry(bid).or_default();
            let text_key = syncplan::norm(&n.text);
            // the same words in another colour or style are another highlight
            let key = format!(
                "{text_key}|{}|{}",
                n.color.as_deref().unwrap_or(""),
                n.style.as_deref().unwrap_or("")
            );
            if known.contains(&key) {
                // a second device copy of the same quote: keep the one with the note
                if !n.note.trim().is_empty() {
                    if let Some(i) = bp
                        .highlights_in
                        .iter_mut()
                        .find(|h| syncplan::norm(&h.note.text) == text_key)
                    {
                        if i.note.note.trim().is_empty() {
                            i.note.note = n.note.clone();
                        }
                    }
                }
                continue;
            }
            known.push(key);
            match locator.locate(bid, &n.text) {
                Some((spine, start, end)) => bp.highlights_in.push(Incoming {
                    note: n.clone(),
                    spine,
                    start,
                    end,
                }),
                None => {
                    // never at a guessed place: a note without a place, kept with its text
                    let (spine, start, end) = crate::core::anchor::unplaced(0);
                    bp.highlights_in.push(Incoming {
                        note: n.clone(),
                        spine,
                        start,
                        end,
                    });
                    bp.unplaced += 1;
                }
            }
        }
        // colour and style: which library highlight each device row is, what the device changed,
        // and rows that are a second highlight of words the library has in another colour
        let mut claimed = HashSet::new();
        // longest first: a passage claims its own library highlight before a shorter quote that is
        // part of it can (the order of the device's rows says nothing about which of the two the
        // library has); a stable sort keeps the device's order among rows of the same length
        let mut ordered: Vec<&DeviceNote> = reading.notes.iter().collect();
        ordered.sort_by_key(|n| std::cmp::Reverse(syncplan::norm(&n.text).len()));
        for n in ordered {
            if identity(&n.text).is_empty() {
                continue;
            }
            if n.id
                .as_ref()
                .is_some_and(|id| book.highlights.iter().any(|h| h.id == *id))
            {
                // the device kept our own id: that is the highlight itself
                claimed.extend(n.id.clone());
                continue;
            }
            let want = crate::look_sync::want_of(&n.color, &n.style);
            match crate::look_sync::pick(
                &book.highlights,
                &book.looks,
                &mut claimed,
                &n.text,
                &want,
            ) {
                crate::look_sync::Pick::Existing(l) => {
                    if !bp.looks_in.iter().any(|x| x.id == l.id) {
                        bp.looks_in.push(l);
                    }
                }
                crate::look_sync::Pick::New => {
                    let represented = bp.highlights_in.iter().any(|i| {
                        identity(&i.note.text) == identity(&n.text)
                            && crate::look_sync::want_of(&i.note.color, &i.note.style) == want
                    });
                    if !represented {
                        let (spine, start, end) =
                            locator.locate(bid, &n.text).unwrap_or_else(|| {
                                bp.unplaced += 1;
                                crate::core::anchor::unplaced(0)
                            });
                        bp.highlights_in.push(Incoming {
                            note: n.clone(),
                            spine,
                            start,
                            end,
                        });
                    }
                }
            }
        }
        // notes the library lacks on highlights it has
        for e in b.enrich.iter().filter(|e| e.to_side == Side::Library) {
            let Some(h) = index_of(&e.annot).and_then(|i| book.highlights.get(i)) else {
                continue;
            };
            if h.note.trim().is_empty()
                && !e.annot.note.trim().is_empty()
                && !bp.notes_in.iter().any(|(id, _)| *id == h.id)
            {
                bp.notes_in
                    .push((h.id.clone(), e.annot.note.trim().to_string()));
            }
        }
        for c in &b.conflicts {
            if !bp.conflicts.contains(c) {
                bp.conflicts.push(c.clone());
            }
        }
        // page bookmarks the library has not handled: each with the place the book gives it
        for m in &reading.bookmarks {
            let source = bookmark_source(m);
            let key = crate::bookmark_sync::Store::key(bid, source, &m.id);
            if book.bookmark_keys.contains(&key) || bp.bookmarks_in.iter().any(|i| i.key == key) {
                continue;
            }
            bp.bookmarks_in.push(BookmarkIn {
                place: locator.locate_place(bid, m),
                key,
                mark: m.clone(),
                source: source.to_string(),
            });
        }
        // library -> device
        let missing: Vec<Highlight> = b
            .notes_to_device
            .iter()
            .filter_map(|a| index_of(a).and_then(|i| book.highlights.get(i)).cloned())
            .collect();
        if !missing.is_empty() {
            if reading.accepts_notes {
                bp.out.push(Outgoing {
                    path: reading.path.clone(),
                    highlights: missing,
                });
            } else {
                bp.held_back += missing.len();
            }
        }
    }
    for (bid, mut bp) in by_book {
        let (with_state, behind) = copies.get(&bid).copied().unwrap_or_default();
        if bp.progress.is_some() {
            bp.restarted = None;
        } else if with_state > 0 && behind == with_state && bp.restarted.is_none() {
            if let (Some(f), Some(book)) = (ahead.get(&bid), library.iter().find(|l| l.id == bid)) {
                bp.library_ahead = Some((book.progress, *f));
            }
        }
        out.books.push(bp);
    }
    out
}

/// The counts the user is shown before anything happens.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    /// `(title, from, to)`
    pub progress: Vec<(String, f64, f64)>,
    pub highlights_to_library: usize,
    pub notes_to_library: usize,
    pub bookmarks_to_library: usize,
    /// Of those, the ones whose place in the book is a guess (a chapter, a percentage).
    pub bookmarks_approx: usize,
    pub highlights_to_device: usize,
    pub notes_to_device: usize,
    pub library_ahead: usize,
    pub held_back: usize,
    pub unplaced: usize,
    pub unmatched: usize,
    /// `(title, conflict)`
    pub conflicts: Vec<(String, Conflict)>,
    /// Books whose progress was left alone because the library copy was read more recently.
    pub restarted: usize,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn go(n: usize) -> &'static str {
    if n == 1 {
        "goes"
    } else {
        "go"
    }
}

fn clip(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= n {
        s
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn pct(p: f64) -> u32 {
    (p * 100.0).round() as u32
}

impl Summary {
    /// One line per kind of change, in plain words, ending with the promise.
    pub fn lines(&self) -> Vec<String> {
        let mut v = Vec::new();
        if !self.progress.is_empty() {
            let shown: Vec<String> = self
                .progress
                .iter()
                .take(3)
                .map(|(t, a, b)| format!("{t} {}%→{}%", pct(*a), pct(*b)))
                .collect();
            let more = self.progress.len().saturating_sub(3);
            v.push(format!(
                "{}: progress moves forward from the device ({}{})",
                plural(self.progress.len(), "book", "books"),
                shown.join(", "),
                if more > 0 {
                    format!(", and {more} more")
                } else {
                    String::new()
                }
            ));
        }
        if self.highlights_to_library > 0 {
            v.push(format!(
                "{} {} device→Bookx",
                plural(self.highlights_to_library, "highlight", "highlights"),
                go(self.highlights_to_library)
            ));
        }
        if self.notes_to_library > 0 {
            v.push(format!(
                "{} {} device→Bookx",
                plural(self.notes_to_library, "note", "notes"),
                go(self.notes_to_library)
            ));
        }
        if self.bookmarks_to_library > 0 {
            v.push(format!(
                "{} {} device→Bookx{}",
                plural(self.bookmarks_to_library, "bookmark", "bookmarks"),
                go(self.bookmarks_to_library),
                if self.bookmarks_approx > 0 {
                    format!(
                        " ({} placed roughly, by chapter or page)",
                        self.bookmarks_approx
                    )
                } else {
                    String::new()
                }
            ));
        }
        if self.highlights_to_device > 0 {
            v.push(format!(
                "{} {} Bookx→device{}",
                plural(self.highlights_to_device, "highlight", "highlights"),
                go(self.highlights_to_device),
                if self.notes_to_device > 0 {
                    format!(" ({} with notes)", self.notes_to_device)
                } else {
                    String::new()
                }
            ));
        }
        if self.library_ahead > 0 {
            v.push(format!(
                "{} further along in Bookx than on the device; the device keeps its own progress",
                plural(self.library_ahead, "book is", "books are"),
            ));
        }
        if self.held_back > 0 {
            v.push(format!(
                "{} {} in Bookx: the device copy keeps a notes file of its own, which is not touched",
                plural(self.held_back, "highlight", "highlights"),
                if self.held_back == 1 { "stays" } else { "stay" },
            ));
        }
        if self.restarted > 0 {
            v.push(format!(
                "{} read more recently in Bookx than the device is ahead; progress is left alone",
                plural(self.restarted, "book was", "books were"),
            ));
        }
        if !self.conflicts.is_empty() {
            v.push(format!(
                "{} a different note on the two sides; both versions are kept (Bookx keeps its note, the device keeps its own, nothing is overwritten)",
                plural(self.conflicts.len(), "highlight has", "highlights have"),
            ));
            for (t, c) in self.conflicts.iter().take(3) {
                v.push(format!(
                    "  • {t}: “{}” — Bookx: “{}” · device: “{}”",
                    clip(&c.text, 60),
                    clip(&c.library_note, 60),
                    clip(&c.device_note, 60)
                ));
            }
            if self.conflicts.len() > 3 {
                v.push(format!("  • and {} more", self.conflicts.len() - 3));
            }
        }
        if self.unplaced > 0 {
            v.push(format!(
                "{} on the device could not be located in the book and {} added as {} without a place (listed in the notes, not painted on the page)",
                plural(self.unplaced, "highlight", "highlights"),
                if self.unplaced == 1 { "is" } else { "are" },
                if self.unplaced == 1 { "a note" } else { "notes" },
            ));
        }
        if v.is_empty() {
            v.push("Everything is already up to date.".to_string());
        }
        v.push("Nothing is deleted on either side.".to_string());
        v
    }

    pub fn text(&self) -> String {
        self.lines().join("\n")
    }
}

impl Plan {
    pub fn summary(&self) -> Summary {
        let mut s = Summary {
            unmatched: self.unmatched,
            ..Default::default()
        };
        let mut out_seen: HashSet<(u64, String)> = HashSet::new();
        for b in &self.books {
            if let Some(p) = &b.progress {
                s.progress.push((b.title.clone(), p.from, p.to));
            }
            if b.library_ahead.is_some() {
                s.library_ahead += 1;
            }
            s.highlights_to_library += b.highlights_in.len();
            s.notes_to_library += b.notes_in.len();
            s.bookmarks_to_library += b.bookmarks_in.len();
            s.bookmarks_approx += b.bookmarks_in.iter().filter(|i| !i.place.exact).count();
            s.restarted += b.restarted.is_some() as usize;
            s.conflicts
                .extend(b.conflicts.iter().map(|c| (b.title.clone(), c.clone())));
            s.unplaced += b.unplaced;
            s.held_back += b.held_back;
            let ids = b.outgoing_ids();
            for o in &b.out {
                for h in &o.highlights {
                    if ids.contains(h.id.as_str()) && out_seen.insert((b.book_id, h.id.clone())) {
                        s.highlights_to_device += 1;
                        if !h.note.trim().is_empty() {
                            s.notes_to_device += 1;
                        }
                    }
                }
            }
        }
        s
    }

    /// Whether applying the plan changes anything.
    pub fn is_empty(&self) -> bool {
        self.books.iter().all(BookPlan::is_empty)
    }
}

/// What [`apply_to_library`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Applied {
    pub progress: usize,
    pub highlights: usize,
    pub notes: usize,
    /// Device bookmarks added to books, and the ones a book already had at that place.
    pub bookmarks: usize,
    pub bookmarks_linked: usize,
    /// Highlights that took a new colour or style from the device.
    pub recolored: usize,
    /// What the device said the look of highlights is: `(book id, highlight id, look)`, for the
    /// ledger (`devstyle::DeviceLooks`).
    pub ledger: Vec<(u64, String, crate::core::devstyle::Look)>,
}

/// Applies the library side of one book's plan. Progress never goes down, highlights are only
/// added, a note is only written onto a highlight that has none.
pub fn apply_book(book: &mut Book, plan: &BookPlan, now: DateTime<Utc>) -> Applied {
    let mut done = Applied::default();
    if let Some(p) = &plan.progress {
        if p.to > book.progress {
            book.progress = if p.finished {
                1.0
            } else {
                p.to.clamp(0.0, 1.0)
            };
            book.last_read = Some(now);
            done.progress = 1;
        }
    }
    for (id, note) in &plan.notes_in {
        if let Some(h) = book.highlights.iter_mut().find(|h| h.id == *id) {
            if h.note.trim().is_empty() {
                h.note = note.clone();
                done.notes += 1;
            }
        }
    }
    for i in &plan.highlights_in {
        let want = crate::look_sync::want_of(&i.note.color, &i.note.style);
        if book
            .highlights
            .iter()
            .any(|h| same_note(h, &i.note) && crate::look_sync::has_look(h, &want))
        {
            continue;
        }
        let id = i
            .note
            .id
            .clone()
            .unwrap_or_else(|| crate::annotation_ids::incoming_id(book, &i.note.text));
        book.highlights.push(Highlight {
            id,
            spine: i.spine,
            start: i.start,
            end: i.end,
            color: want.0.clone().unwrap_or_else(|| "yellow".into()),
            style: want.1.clone().unwrap_or_default(),
            note: i.note.note.trim().to_string(),
            text: i.note.text.clone(),
            created: i.note.created.unwrap_or(now),
        });
        if want.0.is_some() || want.1.is_some() {
            let h = book.highlights.last().expect("just pushed");
            done.ledger.push((
                book.id,
                h.id.clone(),
                crate::core::devstyle::full_look(&h.color, &h.style),
            ));
        }
        done.highlights += 1;
    }
    done
}

/// Puts the device bookmarks of one book's plan into the book and the cable ledger. Returns
/// `(added, linked to a bookmark that was there)`.
pub fn apply_bookmarks(
    book: &mut Book,
    plan: &BookPlan,
    store: &mut crate::bookmark_sync::Store,
    now: DateTime<Utc>,
) -> (usize, usize) {
    use crate::bookmark_sync::{import, Outcome};
    let (mut added, mut linked) = (0, 0);
    for i in &plan.bookmarks_in {
        match import(
            book,
            &mut store.entries,
            &i.key,
            &i.mark,
            &i.place,
            &i.source,
            now,
        ) {
            Outcome::Added => added += 1,
            Outcome::Linked => linked += 1,
            Outcome::Known => {}
        }
    }
    (added, linked)
}

pub fn apply_to_library(lib: &mut crate::core::library::Library, plan: &Plan) -> Applied {
    let now = Utc::now();
    let mut total = Applied::default();
    let mut before = Vec::new();
    let mut store = crate::bookmark_sync::Store::load(&lib.root);
    let store_before = store.clone();
    for b in &plan.books {
        if let Some(book) = lib.get_mut(b.book_id) {
            let (added, linked) = apply_bookmarks(book, b, &mut store, now);
            total.bookmarks += added;
            total.bookmarks_linked += linked;
            let a = apply_book(book, b, now);
            total.progress += a.progress;
            total.highlights += a.highlights;
            total.notes += a.notes;
            total.ledger.extend(a.ledger);
            let snapshot = book.highlights.clone();
            let (n, ledger) = crate::look_sync::apply(&mut book.highlights, &b.looks_in);
            total
                .ledger
                .extend(ledger.into_iter().map(|(id, l)| (b.book_id, id, l)));
            if n > 0 {
                total.recolored += n;
                before.push((b.book_id, snapshot));
            }
        }
    }
    finish_looks(lib, &total, before);
    if store != store_before {
        let _ = store.save(&lib.root);
    }
    total
}

/// Remembers what the devices said about looks and logs colour changes taken from a device as one
/// undoable step.
pub fn finish_looks(
    lib: &mut crate::core::library::Library,
    total: &Applied,
    before: Vec<(u64, Vec<Highlight>)>,
) {
    if !total.ledger.is_empty() {
        let mut ledger = crate::core::devstyle::DeviceLooks::load(&lib.root);
        for (book, id, look) in &total.ledger {
            ledger.looks.insert(
                crate::core::devstyle::DeviceLooks::key(*book, id),
                look.clone(),
            );
        }
        let _ = ledger.save(&lib.root);
    }
    if total.recolored > 0 {
        lib.record_highlight_edit(
            format!(
                "Took the colour of {} highlights from the device",
                total.recolored
            ),
            before,
        );
    }
}

/// Hands the outgoing notes to `sink`; returns how many were written and the problems.
pub fn apply_to_device(plan: &Plan, sink: &mut dyn NoteSink) -> (usize, Vec<String>) {
    let (mut written, mut errors) = (0, Vec::new());
    for b in &plan.books {
        for o in &b.out {
            match sink.write(&o.path, &b.title, &o.highlights) {
                Ok(n) => written += n,
                Err(e) => errors.push(format!("{}: {e:#}", b.title)),
            }
        }
    }
    (written, errors)
}

// -- the header bar and the connect banner -----------------------------------------------------

/// The label of the header-bar Device button: `None` (hidden) without a device, else the name of
/// the first device, with how many more there are.
pub fn button_label(devices: &[bookx_core::contracts::DeviceInfo]) -> Option<String> {
    let first = devices.first()?;
    Some(match devices.len() {
        1 => first.name.clone(),
        n => format!("{} +{}", first.name, n - 1),
    })
}

/// The devices that were not there before.
pub fn newly_connected(
    before: &[bookx_core::contracts::DeviceInfo],
    now: &[bookx_core::contracts::DeviceInfo],
) -> Vec<bookx_core::contracts::DeviceInfo> {
    now.iter()
        .filter(|d| !before.iter().any(|b| b.id == d.id))
        .cloned()
        .collect()
}

/// The title of the `AdwBanner` shown when a device is plugged in.
pub fn banner_title() -> String {
    tr(BANNER_TITLE)
}

/// The banner's only action: a one-shot sync, never a change to the auto-sync setting.
pub fn banner_button_label() -> String {
    tr(BANNER_BUTTON)
}

// The remaining helpers are shared with the Devices window (applied in the corresponding change).
/// Kept for callers that still want one string; the banner itself has no device name and no
/// "Always" or "Not now" choice any more.
#[allow(dead_code)]
pub fn banner_text(_device: &str) -> String {
    banner_title()
}

/// The suggested button under "Books to send" (header caps, LIB-D3).
#[allow(dead_code)]
pub fn send_button_label(n: usize) -> String {
    ntr("Send {} Book", "Send {} Books", n)
}

/// The destructive button under "Books on the device". The count belongs to the confirmation,
/// the button itself always names the action.
#[allow(dead_code)]
pub fn remove_button_label(_n: usize) -> String {
    tr("Remove from Device")
}

/// Confirmation for a removal from the device (it cannot be undone there).
#[allow(dead_code)]
pub fn remove_confirmation_title(n: usize) -> String {
    ntr(
        "Remove {} Book from the Device?",
        "Remove {} Books from the Device?",
        n,
    )
}

/// Title and description of the Devices window when nothing is plugged in.
#[allow(dead_code)]
pub fn no_device_status_title() -> String {
    tr(NO_DEVICE_TITLE)
}

#[allow(dead_code)]
pub fn no_device_status_description() -> String {
    tr(NO_DEVICE_DESCRIPTION)
}

/// An individual device row: the name is the title, the storage line its subtitle.
#[allow(dead_code)]
pub fn device_row_text(name: &str, storage: &str) -> (String, String) {
    (name.to_string(), storage.to_string())
}

/// `(select all, clear selection, remove)` for the "Books on the device" controls.
#[allow(dead_code)]
pub fn selection_controls_state(len: usize, selected: usize) -> (bool, bool, bool) {
    (len > 0, selected > 0, selected > 0)
}

/// The single view toggle: `(icon, accessible name)`.
#[allow(dead_code)]
pub fn view_toggle_state(is_list: bool) -> (&'static str, String) {
    if is_list {
        ("view-list-symbolic", tr(LIST_VIEW_NAME))
    } else {
        ("view-grid-symbolic", tr(GRID_VIEW_NAME))
    }
}

// -- reading the device and writing to it (glue) ---------------------------------------------

/// Notes as the device reader (`bookx_serve::reading`) finds them: Kobo database, KOReader sidecars.
fn note_of(n: &bookx_serve::reading::DeviceNote) -> DeviceNote {
    // the creation time of the highlight (KOReader's `datetime` never changes); the time of the last edit
    // only when the device says nothing else
    let when = n
        .created
        .as_deref()
        .or(n.updated.as_deref())
        .and_then(bookx_serve::reading::parse_time)
        .and_then(|t| DateTime::from_timestamp(t, 0));
    let mut note = n.note.trim().to_string();
    if note == n.text.trim() {
        // Bookx writes the quote into `notes` when there is no note
        note.clear();
    }
    DeviceNote {
        id: None,
        color: n.color.clone(),
        style: n.style.clone(),
        text: n.text.clone(),
        note,
        created: when,
    }
}

/// Where a device bookmark came from: a dog-ear names its chapter file, KOReader's does not.
pub fn bookmark_source(m: &crate::bookmark_sync::DeviceMark) -> &'static str {
    if m.href.is_some() {
        "Kobo"
    } else {
        "KOReader"
    }
}

fn mark_of(b: &bookx_serve::reading::DeviceBookmark) -> crate::bookmark_sync::DeviceMark {
    crate::bookmark_sync::DeviceMark {
        id: b.id.clone(),
        note: b.note.clone(),
        chapter: b.chapter.clone().unwrap_or_default(),
        pos: b.pos.clone(),
        page: b.page,
        percentage: b.percentage,
        href: b.href.clone(),
        chapter_progress: b.chapter_progress,
        created: b
            .created
            .as_deref()
            .and_then(bookx_serve::reading::parse_time)
            .unwrap_or(0),
    }
}

/// Reads the reading state and notes of every book of `device`.
pub fn read_device(
    drivers: &crate::devices::Drivers,
    device: &bookx_core::contracts::DeviceInfo,
) -> anyhow::Result<Vec<DeviceReading>> {
    use bookx_serve::devices::DeviceKind;
    let books = drivers.books(device)?;
    let local = matches!(
        drivers.backend(device),
        crate::devices::Backend::Usb | crate::devices::Backend::Mtp
    );
    let data = if local {
        bookx_serve::reading::collect_data(&device.mount, &books)
    } else {
        Vec::new()
    };
    let sidecars = local && drivers.kind(device) != Some(DeviceKind::Kindle);
    Ok(books
        .iter()
        .map(|b| {
            let d = data.iter().find(|d| d.path == b.path);
            DeviceReading {
                path: b.path.clone(),
                state: d.and_then(|d| d.state.clone()),
                notes: d
                    .map(|d| {
                        d.notes
                            .iter()
                            .filter(|n| !n.bookmark)
                            .map(note_of)
                            .collect()
                    })
                    .unwrap_or_default(),
                accepts_notes: sidecars && !bookx_serve::reading::sidecar_file(&b.path).exists(),
                bookmarks: d
                    .map(|d| d.bookmarks.iter().map(mark_of).collect())
                    .unwrap_or_default(),
            }
        })
        .collect())
}

/// Writes highlights as a new KOReader sidecar; an existing one is never touched.
pub struct SidecarSink;

impl NoteSink for SidecarSink {
    fn write(
        &mut self,
        path: &Path,
        title: &str,
        highlights: &[Highlight],
    ) -> anyhow::Result<usize> {
        let file = bookx_serve::reading::sidecar_file(path);
        if file.exists() || highlights.is_empty() {
            return Ok(0);
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let notes = crate::send_notes::Notes::new(highlights, &[]);
        std::fs::write(
            &file,
            crate::send_notes::koreader_sidecar(title, &[], &notes),
        )?;
        Ok(highlights.len())
    }
}

/// Finds a quote in the text of the book's chapters: character offsets in the reader's text of
/// the chapter ([`crate::core::doc::Doc::reader_text`], the numbers highlights are stored with and
/// the reader paints by), found by the quote itself (see [`crate::core::anchor`]).
pub struct BookLocator {
    pub root: PathBuf,
    pub files: BTreeMap<u64, String>,
    opened: std::sync::Mutex<BTreeMap<u64, std::sync::Arc<OpenedBook>>>,
}

/// A book with its chapters parsed once for a whole import or sync.
struct OpenedBook {
    chapters: crate::core::anchor::BookChapters,
    /// `(normalised title, spine index)` of the contents entries.
    toc: Vec<(String, usize)>,
}

impl BookLocator {
    pub fn new(root: PathBuf, files: BTreeMap<u64, String>) -> BookLocator {
        BookLocator {
            root,
            files,
            opened: Default::default(),
        }
    }

    fn open(&self, book_id: u64) -> Option<std::sync::Arc<OpenedBook>> {
        if let Some(b) = self.opened.lock().ok()?.get(&book_id) {
            return Some(b.clone());
        }
        let file = self.files.get(&book_id)?;
        let book = crate::core::book::OpenBook::open(&self.root.join(file)).ok()?;
        let toc_entries = book.toc();
        let chapters = crate::core::anchor::BookChapters::new(book);
        let toc = crate::core::toc::flatten(&toc_entries)
            .iter()
            .filter_map(|t| {
                let path = t.href.split('#').next().unwrap_or(&t.href);
                Some((key(&t.title), chapters.index_of(path)?))
            })
            .filter(|(k, _)| !k.is_empty())
            .collect();
        let opened = std::sync::Arc::new(OpenedBook { chapters, toc });
        self.opened.lock().ok()?.insert(book_id, opened.clone());
        Some(opened)
    }
}

impl crate::bookmark_sync::Geometry for OpenedBook {
    fn count(&self) -> usize {
        use crate::core::anchor::Chapters;
        self.chapters.count()
    }

    fn weight(&self, spine: usize) -> f64 {
        use crate::core::anchor::Chapters;
        self.chapters
            .texts(spine)
            .first()
            .map_or(0.0, |t| t.len() as f64)
    }

    fn xhtml(&self, spine: usize) -> Option<String> {
        use crate::highlight_goto::Source;
        Source::xhtml(&self.chapters, spine)
    }

    fn spine_of_title(&self, key: &str) -> Option<usize> {
        self.toc.iter().find(|(t, _)| t == key).map(|(_, s)| *s)
    }

    fn spine_of_file(&self, href: &str) -> Option<usize> {
        let href = href.trim_start_matches('/');
        self.chapters.book().spine_paths().iter().position(|p| {
            p == href || p.ends_with(&format!("/{href}")) || href.ends_with(&format!("/{p}"))
        })
    }
}

impl Locator for BookLocator {
    fn locate(&self, book_id: u64, text: &str) -> Option<(usize, i32, i32)> {
        self.locate_in(book_id, text, None)
    }

    fn locate_place(
        &self,
        book_id: u64,
        mark: &crate::bookmark_sync::DeviceMark,
    ) -> crate::bookmark_sync::Place {
        match self.open(book_id) {
            Some(opened) => crate::bookmark_sync::resolve_place(&*opened, mark),
            None => crate::bookmark_sync::Place::unknown(),
        }
    }

    fn locate_in(
        &self,
        book_id: u64,
        text: &str,
        chapter: Option<&str>,
    ) -> Option<(usize, i32, i32)> {
        let opened = self.open(book_id)?;
        let hint = chapter
            .map(key)
            .filter(|k| !k.is_empty())
            .and_then(|k| opened.toc.iter().find(|(t, _)| *t == k).map(|(_, s)| *s));
        let r = crate::core::anchor::resolve(&opened.chapters, text, hint, None)?;
        Some((r.spine, r.found.start as i32, r.found.end as i32))
    }

    fn locate_hinted(
        &self,
        book_id: u64,
        text: &str,
        chapter: Option<&str>,
        hint: &crate::highlight_goto::Hint,
    ) -> Option<(usize, i32, i32)> {
        if hint.pos0.trim().is_empty() && hint.pos1.trim().is_empty() {
            return self.locate_in(book_id, text, chapter);
        }
        let opened = self.open(book_id)?;
        let (spine, start, end) = crate::core::anchor::unplaced(0);
        let h = Highlight {
            id: String::new(),
            spine,
            start,
            end,
            color: String::new(),
            style: String::new(),
            note: String::new(),
            text: text.to_string(),
            created: Utc::now(),
        };
        let mut hint = hint.clone();
        if hint.chapter.trim().is_empty() {
            hint.chapter = chapter.unwrap_or_default().to_string();
        }
        match crate::highlight_goto::resolve(&opened.chapters, &h, Some(&hint), &opened.toc) {
            crate::highlight_goto::Outcome::Found(t) => {
                Some((t.spine, t.start as i32, t.end as i32))
            }
            _ => None,
        }
    }

    fn locate_other(
        &self,
        book_id: u64,
        text: &str,
        chapter: Option<&str>,
        taken: &[(usize, i32, i32)],
    ) -> Option<(usize, i32, i32)> {
        let opened = self.open(book_id)?;
        let hint = chapter
            .map(key)
            .filter(|k| !k.is_empty())
            .and_then(|k| opened.toc.iter().find(|(t, _)| *t == k).map(|(_, s)| *s));
        let r = crate::core::anchor::resolve_avoiding(&opened.chapters, text, hint, taken)?;
        Some((r.spine, r.found.start as i32, r.found.end as i32))
    }
}

/// A locator for the library's books.
pub fn book_locator(lib: &crate::core::library::Library) -> BookLocator {
    BookLocator::new(
        lib.root.clone(),
        lib.books.iter().map(|b| (b.id, b.file.clone())).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// A locator that finds nothing (device notes are then reported, not imported).
    struct NoLocator;

    impl Locator for NoLocator {
        fn locate(&self, _: u64, _: &str) -> Option<(usize, i32, i32)> {
            None
        }
    }

    fn hl(id: &str, text: &str, note: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start: 0,
            end: text.len() as i32,
            color: "yellow".into(),
            style: String::new(),
            note: note.into(),
            text: text.into(),
            created: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
        }
    }

    fn dn(text: &str, note: &str) -> DeviceNote {
        DeviceNote {
            id: None,
            text: text.into(),
            note: note.into(),
            created: None,
            color: None,
            style: None,
        }
    }

    fn lib_book(id: u64, title: &str, progress: f64, hs: Vec<Highlight>) -> LibraryBook {
        LibraryBook {
            id,
            title: title.into(),
            authors: vec![],
            progress,
            last_read: None,
            highlights: hs,
            looks: Default::default(),
            bookmark_keys: Default::default(),
        }
    }

    fn reading(file: &str, progress: f64, notes: Vec<DeviceNote>, accepts: bool) -> DeviceReading {
        DeviceReading {
            path: PathBuf::from(format!("/dev/{file}")),
            state: Some(DeviceState {
                title: None,
                progress,
                status: if progress >= 1.0 {
                    Status::Finished
                } else {
                    Status::Reading
                },
                last_read: None,
                source: "Kobo",
            }),
            notes,
            bookmarks: vec![],
            accepts_notes: accepts,
        }
    }

    struct Fixed(Option<(usize, i32, i32)>);
    impl Locator for Fixed {
        fn locate(&self, _: u64, _: &str) -> Option<(usize, i32, i32)> {
            self.0
        }
    }

    #[test]
    fn progress_moves_forward_only() {
        let lib = [
            lib_book(1, "Dune", 0.42, vec![]),
            lib_book(2, "Emma", 0.9, vec![]),
        ];
        let dev = [
            reading("Dune.kepub.epub", 0.67, vec![], true),
            reading("Emma.epub", 0.3, vec![], true),
        ];
        let p = plan(&dev, &lib, &NoLocator);
        let dune = &p.books[0];
        assert_eq!(
            dune.progress.as_ref().map(|m| (m.from, m.to)),
            Some((0.42, 0.67))
        );
        let emma = &p.books[1];
        assert!(emma.progress.is_none());
        assert_eq!(emma.library_ahead, Some((0.9, 0.3)));
        let s = p.summary();
        assert_eq!(s.progress.len(), 1);
        assert_eq!(s.library_ahead, 1);
        assert!(s.lines()[0].contains("Dune 42%→67%"));
    }

    #[test]
    fn books_are_matched_by_title_with_a_file_name_prefix_and_two_copies() {
        let lib = [
            lib_book(1, "Gamma Rays", 0.0, vec![]),
            lib_book(2, "Delta", 0.0, vec![]),
        ];
        let mut dev = [
            reading("01 - Gamma Rays.kepub.epub", 0.2, vec![], true),
            reading("Delta.epub", 0.4, vec![], true),
            reading("Delta (1).epub", 1.0, vec![], true),
            reading("Unknown.epub", 1.0, vec![], true),
        ];
        // the device says what the second copy is called, whatever the file name
        dev[2].state.as_mut().unwrap().title = Some("Delta".into());
        let p = plan(&dev, &lib, &NoLocator);
        let moves: Vec<(u64, f64)> = p
            .books
            .iter()
            .map(|b| (b.book_id, b.progress.as_ref().unwrap().to))
            .collect();
        assert_eq!(moves, vec![(1, 0.2), (2, 1.0)]);
        assert_eq!(p.unmatched, 1);
    }

    #[test]
    fn a_small_difference_is_not_a_change() {
        let lib = [lib_book(1, "Dune", 0.5, vec![])];
        let p = plan(
            &[reading("Dune.epub", 0.503, vec![], true)],
            &lib,
            &NoLocator,
        );
        assert!(p.is_empty());
    }

    #[test]
    fn progress_never_goes_down_even_when_applied_blindly() {
        let mut b = book(1, "Dune", 0.8, vec![]);
        let bp = BookPlan {
            book_id: 1,
            progress: Some(ProgressMove {
                from: 0.1,
                to: 0.3,
                finished: false,
                source: "Kobo",
            }),
            ..Default::default()
        };
        let done = apply_book(&mut b, &bp, Utc::now());
        assert_eq!(done.progress, 0);
        assert_eq!(b.progress, 0.8);
    }

    fn book(id: u64, title: &str, progress: f64, hs: Vec<Highlight>) -> Book {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Book {
            id,
            file: format!("{title}.epub"),
            cover: None,
            meta: crate::core::opf::Metadata {
                title: title.into(),
                ..Default::default()
            },
            added: now,
            modified: now,
            last_read: None,
            size: 1,
            progress,
            position: None,
            bookmarks: vec![],
            highlights: hs,
            formats: vec![],
            resources: vec![],
            drm: String::new(),
            custom: Default::default(),
            marked: false,
        }
    }

    #[test]
    fn the_furthest_device_copy_wins_and_finished_means_one() {
        let lib = [lib_book(1, "Dune", 0.1, vec![])];
        let dev = [
            reading("a/Dune.epub", 0.4, vec![], true),
            reading("b/Dune.kepub.epub", 1.0, vec![], true),
        ];
        let p = plan(&dev, &lib, &NoLocator);
        let m = p.books[0].progress.clone().unwrap();
        assert_eq!(m.to, 1.0);
        assert!(m.finished);
        let mut b = book(1, "Dune", 0.1, vec![]);
        apply_book(&mut b, &p.books[0], Utc::now());
        assert_eq!(b.progress, 1.0);
        assert!(b.last_read.is_some());
    }

    #[test]
    fn highlights_are_united_by_identity_not_duplicated() {
        let lib = [lib_book(
            1,
            "Dune",
            0.5,
            vec![hl("a", "Fear is the mind-killer.", "")],
        )];
        let dev = [reading(
            "Dune.epub",
            0.5,
            // same quote with other case and punctuation, plus a new one
            vec![
                dn("fear is the MIND killer", ""),
                dn("I must not fear.", ""),
            ],
            true,
        )];
        let p = plan(&dev, &lib, &Fixed(Some((2, 10, 26))));
        let b = &p.books[0];
        assert_eq!(b.highlights_in.len(), 1);
        assert_eq!(b.highlights_in[0].note.text, "I must not fear.");
        assert_eq!(
            (b.highlights_in[0].spine, b.highlights_in[0].start),
            (2, 10)
        );
        assert!(b.out.is_empty());
        let mut bk = book(
            1,
            "Dune",
            0.5,
            vec![hl("a", "Fear is the mind-killer.", "")],
        );
        let done = apply_book(&mut bk, b, Utc::now());
        assert_eq!(done.highlights, 1);
        assert_eq!(bk.highlights.len(), 2);
        // applying again adds nothing
        let again = apply_book(&mut bk, b, Utc::now());
        assert_eq!(again.highlights, 0);
        assert_eq!(bk.highlights.len(), 2);
    }

    /// Root cause R5 (docs/highlight-sync-rca.md): the device rows were matched in file order, so a short
    /// quote that lies inside a longer highlight (marked in another colour) took the longer
    /// highlight's place and gave it its colour; the long row then found nothing and was added
    /// again. The longest row must claim its own highlight first.
    #[test]
    fn a_short_quote_inside_a_longer_highlight_does_not_take_its_colour() {
        let long = "Service discovery is how services find each other at runtime without hard coded addresses";
        let short = "services find each other at runtime";
        let lib = [lib_book(1, "Dune", 0.5, vec![hl("a", long, "")])];
        let mut s = dn(short, "");
        s.color = Some("purple".into());
        let mut l = dn(long, "");
        l.color = Some("yellow".into());
        l.style = Some("background".into());
        // the short row comes first in the file
        let p = plan(
            &[reading("Dune.epub", 0.5, vec![s, l], true)],
            &lib,
            &Fixed(Some((2, 10, 26))),
        );
        let mut bk = book(1, "Dune", 0.5, vec![hl("a", long, "")]);
        let done = apply_book(&mut bk, &p.books[0], Utc::now());
        let (n, _) = crate::look_sync::apply(&mut bk.highlights, &p.books[0].looks_in);
        assert_eq!(n, 0, "the long highlight keeps its own colour");
        assert_eq!(done.highlights, 1, "only the short quote is new");
        let by = |t: &str| bk.highlights.iter().find(|h| h.text == t).unwrap();
        assert_eq!(by(long).color, "yellow");
        assert_eq!(by(short).color, "purple");
        assert_eq!(bk.highlights.len(), 2);
    }

    #[test]
    fn the_same_id_is_the_same_highlight() {
        let lib = [lib_book(
            1,
            "Dune",
            0.5,
            vec![hl("a", "original words", "")],
        )];
        let mut n = dn("reworded by the device", "");
        n.id = Some("a".into());
        let p = plan(
            &[reading("Dune.epub", 0.5, vec![n], true)],
            &lib,
            &Fixed(Some((0, 0, 1))),
        );
        assert!(p.is_empty());
    }

    #[test]
    fn the_richer_note_wins_and_an_existing_note_is_never_overwritten() {
        let lib = [lib_book(
            1,
            "Dune",
            0.5,
            vec![
                hl("a", "no note here", ""),
                hl("b", "my own note here", "mine"),
            ],
        )];
        let dev = [reading(
            "Dune.epub",
            0.5,
            vec![
                dn("no note here", "device note"),
                dn("my own note here", "theirs"),
            ],
            true,
        )];
        let p = plan(&dev, &lib, &NoLocator);
        let b = &p.books[0];
        assert_eq!(
            b.notes_in,
            vec![("a".to_string(), "device note".to_string())]
        );
        let mut bk = book(
            1,
            "Dune",
            0.5,
            vec![
                hl("a", "no note here", ""),
                hl("b", "my own note here", "mine"),
            ],
        );
        let done = apply_book(&mut bk, b, Utc::now());
        assert_eq!(done.notes, 1);
        assert_eq!(bk.highlights[0].note, "device note");
        assert_eq!(bk.highlights[1].note, "mine");
        assert_eq!(p.summary().notes_to_library, 1);
    }

    #[test]
    fn library_highlights_go_to_a_device_that_lacks_them() {
        let lib = [lib_book(
            1,
            "Dune",
            0.5,
            vec![
                hl("a", "kept on both", "n1"),
                hl("b", "only in bookx", "n2"),
                hl("c", "plain one", ""),
            ],
        )];
        let dev = [reading(
            "Dune.epub",
            0.5,
            vec![dn("kept on both", "n1")],
            true,
        )];
        let p = plan(&dev, &lib, &NoLocator);
        let b = &p.books[0];
        assert_eq!(b.out.len(), 1);
        assert_eq!(b.out[0].highlights.len(), 2);
        let s = p.summary();
        assert_eq!((s.highlights_to_device, s.notes_to_device), (2, 1));
        assert!(s
            .text()
            .contains("2 highlights go Bookx→device (1 with notes)"));
    }

    #[test]
    fn a_device_copy_with_a_notes_file_is_not_written_to() {
        let lib = [lib_book(1, "Dune", 0.5, vec![hl("b", "only in bookx", "")])];
        let dev = [reading("Dune.epub", 0.5, vec![dn("other", "")], false)];
        let p = plan(&dev, &lib, &Fixed(None));
        assert!(p.books[0].out.is_empty());
        assert_eq!(p.books[0].held_back, 1);
        assert_eq!(p.books[0].unplaced, 1);
        let s = p.summary();
        assert_eq!((s.held_back, s.unplaced, s.highlights_to_device), (1, 1, 0));
    }

    #[test]
    fn nothing_is_ever_removed_from_the_library() {
        // the device has no notes at all: the library keeps its own
        let lib = [lib_book(
            1,
            "Dune",
            0.5,
            vec![hl("a", "mine", "x"), hl("b", "mine too", "")],
        )];
        let p = plan(&[reading("Dune.epub", 0.2, vec![], true)], &lib, &NoLocator);
        let mut bk = book(1, "Dune", 0.5, lib[0].highlights.clone());
        apply_book(&mut bk, &p.books[0], Utc::now());
        assert_eq!(bk.highlights.len(), 2);
        assert_eq!(bk.progress, 0.5);
        // and the device is only ever added to
        assert_eq!(p.books[0].out[0].highlights.len(), 2);
    }

    #[test]
    fn unknown_device_books_are_counted_not_invented() {
        let lib = [lib_book(1, "Dune", 0.0, vec![])];
        let p = plan(
            &[reading("Something Else.epub", 0.5, vec![], true)],
            &lib,
            &NoLocator,
        );
        assert_eq!((p.matched, p.unmatched), (0, 1));
        assert!(p.books.is_empty());
        assert_eq!(p.summary().unmatched, 1);
    }

    #[test]
    fn two_device_copies_of_one_quote_import_it_once_with_the_note() {
        let lib = [lib_book(1, "Dune", 0.5, vec![])];
        let dev = [
            reading("a/Dune.epub", 0.5, vec![dn("a long quote here", "")], true),
            reading(
                "b/Dune.epub",
                0.5,
                vec![dn("A long quote here!", "with note")],
                true,
            ),
        ];
        let p = plan(&dev, &lib, &Fixed(Some((0, 1, 18))));
        assert_eq!(p.books[0].highlights_in.len(), 1);
        assert_eq!(p.books[0].highlights_in[0].note.note, "with note");
    }

    #[test]
    fn different_notes_on_the_same_highlight_are_reported_and_nothing_is_overwritten() {
        let lib = [lib_book(
            1,
            "Dune",
            0.5,
            vec![hl("b", "my own note here", "mine")],
        )];
        let dev = [reading(
            "Dune.epub",
            0.5,
            vec![dn("my own note here", "theirs")],
            true,
        )];
        let p = plan(&dev, &lib, &NoLocator);
        assert_eq!(p.summary().conflicts.len(), 1);
        assert!(p.is_empty(), "a conflict changes nothing");
        let s = p.summary();
        let text = s.text();
        assert!(text.contains("1 highlight has a different note on the two sides"));
        assert!(text.contains("Bookx: “mine” · device: “theirs”"));
        assert!(text.contains("nothing is overwritten"));
        let mut bk = book(1, "Dune", 0.5, lib[0].highlights.clone());
        apply_book(&mut bk, &p.books[0], Utc::now());
        assert_eq!(bk.highlights[0].note, "mine");
    }

    #[test]
    fn a_book_read_again_in_bookx_is_not_pulled_forward_by_an_old_device_position() {
        let mut l = lib_book(1, "Dune", 0.1, vec![]);
        l.last_read = Some(1_800_000_000);
        let mut r = reading("Dune.epub", 0.8, vec![], true);
        r.state.as_mut().unwrap().last_read = Some("2020-01-01 00:00:00".into());
        let p = plan(&[r], &[l], &NoLocator);
        assert!(p.books[0].progress.is_none());
        assert_eq!(p.books[0].restarted, Some((0.1, 0.8)));
        assert_eq!(p.summary().restarted, 1);
        assert!(p.summary().text().contains("read more recently in Bookx"));
    }

    #[test]
    fn authors_decide_between_two_books_with_the_same_title() {
        let mut a = lib_book(1, "Dune", 0.0, vec![]);
        a.authors = vec!["Frank Herbert".into()];
        let mut b = lib_book(2, "Dune", 0.0, vec![]);
        b.authors = vec!["Someone Else".into()];
        let ambiguous = plan(
            &[reading("Dune.epub", 0.5, vec![], true)],
            &[a, b],
            &NoLocator,
        );
        assert_eq!((ambiguous.matched, ambiguous.unmatched), (0, 1));
    }

    #[test]
    fn the_summary_says_what_goes_where() {
        let mut s = Summary::default();
        for (t, a, b) in [
            ("A", 0.42, 0.67),
            ("B", 0.0, 0.5),
            ("C", 0.1, 0.2),
            ("D", 0.1, 0.9),
        ] {
            s.progress.push((t.to_string(), a, b));
        }
        s.highlights_to_library = 5;
        s.highlights_to_device = 2;
        s.notes_to_device = 2;
        let text = s.text();
        assert!(text.contains("4 books: progress moves forward from the device (A 42%→67%"));
        assert!(text.contains("and 1 more"));
        assert!(text.contains("5 highlights go device→Bookx"));
        assert!(text.contains("2 highlights go Bookx→device (2 with notes)"));
        assert!(text.ends_with("Nothing is deleted on either side."));
        assert!(Summary::default().text().contains("already up to date"));
    }

    #[test]
    fn device_notes_are_converted_and_a_quote_in_notes_is_no_note() {
        let n = bookx_serve::reading::DeviceNote {
            text: "Quoted words".into(),
            note: "Quoted words".into(),
            chapter: None,
            created: Some("2026-05-06 07:08:09".into()),
            updated: None,
            color: None,
            style: None,
            bookmark: false,
        };
        let d = note_of(&n);
        assert_eq!(d.note, "");
        assert_eq!(
            d.created,
            Some(Utc.with_ymd_and_hms(2026, 5, 6, 7, 8, 9).unwrap())
        );
        let mine = bookx_serve::reading::DeviceNote {
            note: " my note ".into(),
            ..n
        };
        assert_eq!(note_of(&mine).note, "my note");
    }

    #[test]
    fn our_own_sidecar_reads_back_as_the_same_notes() {
        let hs = vec![
            hl("a", "first quote", "with note"),
            hl("b", "second quote", ""),
        ];
        let lua =
            crate::send_notes::koreader_sidecar("T", &[], &crate::send_notes::Notes::new(&hs, &[]));
        let n: Vec<DeviceNote> = bookx_serve::reading::parse_sidecar(&lua)
            .notes
            .iter()
            .map(note_of)
            .collect();
        assert_eq!(n.len(), 2);
        let lib = [lib_book(1, "T", 0.0, hs)];
        let p = plan(&[reading("T.epub", 0.0, n, false)], &lib, &NoLocator);
        assert!(p.is_empty(), "a round trip must be a no-op");
    }

    struct Mem(Vec<(PathBuf, usize)>);
    impl NoteSink for Mem {
        fn write(&mut self, path: &Path, _: &str, h: &[Highlight]) -> anyhow::Result<usize> {
            self.0.push((path.to_path_buf(), h.len()));
            Ok(h.len())
        }
    }

    #[test]
    fn outgoing_notes_reach_the_sink() {
        let lib = [lib_book(
            1,
            "Dune",
            0.0,
            vec![hl("a", "one", "x"), hl("b", "two", "")],
        )];
        let p = plan(&[reading("Dune.epub", 0.0, vec![], true)], &lib, &NoLocator);
        let mut m = Mem(vec![]);
        let (n, errs) = apply_to_device(&p, &mut m);
        assert_eq!((n, errs.len()), (2, 0));
        assert_eq!(m.0[0].0, PathBuf::from("/dev/Dune.epub"));
    }

    #[test]
    fn the_sidecar_sink_never_overwrites() {
        let d = std::env::temp_dir().join(format!("bookx-ds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let book = d.join("Dune.epub");
        let hs = vec![hl("a", "a quote", "")];
        let mut sink = SidecarSink;
        assert_eq!(sink.write(&book, "Dune", &hs).unwrap(), 1);
        let file = bookx_serve::reading::sidecar_file(&book);
        std::fs::write(&file, "device wrote this").unwrap();
        assert_eq!(sink.write(&book, "Dune", &hs).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "device wrote this");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_button_and_banner_follow_the_devices() {
        use bookx_core::contracts::DeviceInfo;
        let d = |id: &str, name: &str| DeviceInfo {
            id: id.into(),
            name: name.into(),
            mount: PathBuf::from("/m").join(id),
        };
        assert_eq!(button_label(&[]), None);
        let kobo = d("k", "Kobo Libra");
        assert_eq!(
            button_label(std::slice::from_ref(&kobo)).as_deref(),
            Some("Kobo Libra")
        );
        let both = [kobo.clone(), d("b", "Boox")];
        assert_eq!(button_label(&both).as_deref(), Some("Kobo Libra +1"));
        assert_eq!(newly_connected(std::slice::from_ref(&kobo), &both).len(), 1);
        assert_eq!(newly_connected(&both, &both).len(), 0);
        assert_eq!(newly_connected(&[], &both).len(), 2);
        // the banner is an AdwBanner: a fixed title, one action and built-in dismiss
        assert_eq!(banner_title(), "E-reader connected");
        assert_eq!(banner_button_label(), "Sync");
        let banner = banner_text("Kobo Libra");
        assert_eq!(banner, banner_title());
        assert!(!banner.contains("Always"), "{banner}");
        assert!(!banner.contains("Not now"), "{banner}");
    }

    #[test]
    fn device_window_labels_and_states() {
        assert_eq!(AUTO_SYNC_TITLE, "Always sync when connected");
        assert_eq!(send_button_label(1), "Send 1 Book");
        assert_eq!(send_button_label(3), "Send 3 Books");
        assert_eq!(remove_button_label(0), "Remove from Device");
        assert_eq!(remove_button_label(2), "Remove from Device");
        assert_eq!(
            remove_confirmation_title(1),
            "Remove 1 Book from the Device?"
        );
        assert_eq!(
            remove_confirmation_title(4),
            "Remove 4 Books from the Device?"
        );
        assert_eq!(no_device_status_title(), "No Device Connected");
        assert_eq!(
            no_device_status_description(),
            "Connect your e-reader with USB, or add a WebDAV folder below."
        );
        assert_eq!(
            device_row_text("Kobo Libra", "Main memory · 29.1 GB free of 32.4 GB"),
            (
                "Kobo Libra".into(),
                "Main memory · 29.1 GB free of 32.4 GB".into()
            )
        );
        assert_eq!(selection_controls_state(0, 0), (false, false, false));
        assert_eq!(selection_controls_state(3, 0), (true, false, false));
        assert_eq!(selection_controls_state(3, 2), (true, true, true));
        assert_eq!(
            view_toggle_state(false),
            ("view-grid-symbolic", "Grid view".into())
        );
        assert_eq!(
            view_toggle_state(true),
            ("view-list-symbolic", "List view".into())
        );
    }

    #[test]
    fn the_explanations_say_what_the_code_does() {
        for needle in [
            "USB",
            "Kobo’s own database",
            "KOReader’s",
            "nothing is deleted",
            "never written",
        ] {
            assert!(AUTO_SYNC_HELP.contains(needle), "{needle}");
        }
        assert!(AUTO_SYNC_SUBTITLE.contains("on every connection"));
        assert!(AUTO_SYNC_HELP.contains("never changes this setting"));
        assert!(!AUTO_SYNC_HELP.contains("you are asked first"));
        assert!(NOTES_SUBTITLE.contains("My notes"));
        // the section title the text promises is the one that is written
        assert_eq!(crate::send_notes::APPENDIX_TITLE, "My notes");
        assert!(NOTES_SUBTITLE.contains("library copy is not changed"));
    }

    #[test]
    fn quotes_are_found_despite_curly_quotes_dashes_and_line_breaks() {
        let hay: Vec<char> =
            "He said, \u{201c}Fear is the\n  mind\u{2014}killer.\u{201d} Then more."
                .chars()
                .collect();
        let find_loose = |hay: &[char], q: &str| {
            crate::core::anchor::locate_exact(hay, q, None).map(|f| (f.start, f.end))
        };
        let (a, b) = find_loose(&hay, "\"Fear is the mind-killer.\"").unwrap();
        let found: String = hay[a..b].iter().collect();
        assert!(
            found.starts_with('\u{201c}') && found.ends_with('\u{201d}'),
            "{found:?}"
        );
        assert!(find_loose(&hay, "Fear is the mind-killer. Then less.").is_none());
        assert!(find_loose(&hay, "short").is_none());
        let soft: Vec<char> = "mind-kil\u{ad}ler is here".chars().collect();
        assert_eq!(find_loose(&soft, "mind-killer is"), Some((0, 15)));
    }

    #[test]
    fn prefs_default_to_manual_sync_and_notes_on() {
        let p = SyncPrefs::default();
        assert!(!p.auto_sync && p.include_notes);
        let q: SyncPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, q);
    }

    /// A locator that gives every device bookmark the same exact place (chapter 3, 40 % in).
    struct PlaceAt(crate::bookmark_sync::Place);

    impl Locator for PlaceAt {
        fn locate(&self, _: u64, _: &str) -> Option<(usize, i32, i32)> {
            None
        }
        fn locate_place(
            &self,
            _: u64,
            _: &crate::bookmark_sync::DeviceMark,
        ) -> crate::bookmark_sync::Place {
            self.0
        }
    }

    fn exact_place(spine: usize, frac: f64) -> PlaceAt {
        PlaceAt(crate::bookmark_sync::Place {
            spine,
            frac,
            how: crate::bookmark_sync::How::Xpointer,
            exact: true,
        })
    }

    fn device_mark(id: &str, page: i64, kobo: bool) -> crate::bookmark_sync::DeviceMark {
        crate::bookmark_sync::DeviceMark {
            id: id.into(),
            page: Some(page),
            href: kobo.then(|| "OEBPS/c3.xhtml".to_string()),
            created: 1_700_000_000,
            ..Default::default()
        }
    }

    fn with_marks(
        marks: Vec<crate::bookmark_sync::DeviceMark>,
    ) -> (Vec<DeviceReading>, Vec<LibraryBook>) {
        let mut r = reading("Dune.epub", 0.5, vec![], true);
        r.bookmarks = marks;
        (vec![r], vec![lib_book(1, "Dune", 0.5, vec![])])
    }

    #[test]
    fn device_bookmarks_are_planned_with_their_place_and_counted_in_the_summary() {
        let (dev, lib) = with_marks(vec![device_mark("a", 7, false)]);
        let p = plan(&dev, &lib, &exact_place(2, 0.4));
        let b = &p.books[0];
        assert_eq!(b.bookmarks_in.len(), 1);
        assert_eq!(
            (
                b.bookmarks_in[0].place.spine,
                b.bookmarks_in[0].source.as_str()
            ),
            (2, "KOReader")
        );
        assert!(!b.is_empty());
        let t = p.summary().text();
        assert!(t.contains("1 bookmark goes device→Bookx"), "{t}");
        assert!(!t.contains("placed roughly"));
        // a place the book could not give is said to be rough
        let p = plan(&dev, &lib, &NoLocator);
        assert!(
            p.summary().text().contains("(1 placed roughly"),
            "{}",
            p.summary().text()
        );
    }

    #[test]
    fn the_cable_sync_adds_a_bookmark_once_and_never_again_after_the_reader_deleted_it() {
        let (dev, mut lib) = with_marks(vec![
            device_mark("a", 7, false),
            device_mark("b", 30, false),
        ]);
        let loc = PlaceAt(crate::bookmark_sync::Place {
            spine: 2,
            frac: 0.4,
            how: crate::bookmark_sync::How::Xpointer,
            exact: true,
        });
        let mut bk = book(1, "Dune", 0.5, vec![]);
        let mut store = crate::bookmark_sync::Store::default();
        let p = plan(&dev, &lib, &loc);
        // the two pages resolve to the same place here: one bookmark, the other is linked to it
        let (added, linked) = apply_bookmarks(&mut bk, &p.books[0], &mut store, Utc::now());
        assert_eq!((added, linked), (1, 1));
        assert_eq!(bk.bookmarks.len(), 1);
        assert_eq!(bk.bookmarks[0].created.timestamp(), 1_700_000_000);
        // the next sync knows both keys: nothing to do
        lib[0].bookmark_keys = store.entries.keys().cloned().collect();
        assert!(plan(&dev, &lib, &loc).books[0].bookmarks_in.is_empty());
        // the reader deletes the bookmark; a sync that does not know the ledger still cannot bring it back
        bk.bookmarks.clear();
        let again = apply_bookmarks(&mut bk, &p.books[0], &mut store, Utc::now());
        assert_eq!(again, (0, 0));
        assert!(bk.bookmarks.is_empty());
    }

    #[test]
    fn a_kobo_dog_ear_and_a_koreader_bookmark_at_one_place_are_one_bookmark() {
        let (dev, lib) = with_marks(vec![device_mark("k", 7, false), device_mark("d", 7, true)]);
        let p = plan(&dev, &lib, &exact_place(1, 0.25));
        let sources: Vec<&str> = p.books[0]
            .bookmarks_in
            .iter()
            .map(|i| i.source.as_str())
            .collect();
        assert_eq!(sources, ["KOReader", "Kobo"]);
        let mut bk = book(1, "Dune", 0.5, vec![]);
        let mut store = crate::bookmark_sync::Store::default();
        assert_eq!(
            apply_bookmarks(&mut bk, &p.books[0], &mut store, Utc::now()),
            (1, 1)
        );
        assert_eq!(bk.bookmarks.len(), 1);
    }

    #[test]
    fn bookmark_keys_come_from_the_cable_ledger() {
        let dir = std::env::temp_dir().join(format!("bookx-ds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut store = crate::bookmark_sync::Store::default();
        let key = crate::bookmark_sync::Store::key(1, "KOReader", "a");
        store.entries.insert(
            key.clone(),
            crate::bookmark_sync::Entry {
                book_id: 1,
                bookmark: "bm-1".into(),
                ..Default::default()
            },
        );
        store.save(&dir).unwrap();
        let mut books = vec![
            lib_book(1, "Dune", 0.5, vec![]),
            lib_book(2, "Other", 0.5, vec![]),
        ];
        attach_looks(&mut books, &dir);
        assert!(books[0].bookmark_keys.contains(&key));
        assert!(books[1].bookmark_keys.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
