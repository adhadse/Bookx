//! KOReader annotation sync, the Bookx side: merging what a KOReader device sent through
//! `bookx_serve::annotation_sync` into the library, and what Bookx has made back to the server.
//! No GTK in here. The wire protocol and the conflict rule are in the crate and in
//! `docs/koreader-annotation-sync.md`.
//!
//! **Rules**
//!
//! * A highlight arrives with its text; **the text decides where it goes**. The xpointers
//!   KOReader sends are kept as hints only. Bookx finds the quote in the book through the
//!   [`Locator`] (the same text-verified search the device sync and the imports use). A quote it
//!   cannot find becomes an *unanchored* highlight (`core::anchor::unplaced`): it keeps its text
//!   and note, is listed in Notes & Highlights as "Could not be located in the book" (with
//!   *Locate again*), and is painted nowhere on a page. Nothing is lost and nothing is misplaced.
//! * A quote the library already has (same text, [`syncplan::same_highlight`]) is linked to the
//!   KOReader annotation instead of added twice; its note is filled in only when it has none.
//! * Every annotation is remembered in a **ledger** (`annotation-sync.json`): which highlight it
//!   became, where it came from (`KOReader (device name)`) and the note both sides had at the
//!   last sync. The ledger is what makes a repeat harmless and what decides conflicts:
//!   * a newer note on KOReader replaces the library note only when the library note is still the
//!     one that was synced (otherwise the library keeps its note and the conflict is counted);
//!   * a deletion on KOReader removes the library highlight only when it was made by this sync and
//!     the reader did not touch its note since; else it stays;
//!   * nothing else is ever deleted.
//! * Highlights made in Bookx go to the server for the books a KOReader device has synced
//!   (nothing is published for books the device never mentioned), under their own ids, and a
//!   highlight deleted in Bookx becomes a tombstone there.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bookx_serve::annotation_sync::{
    Annotation, Change, KIND_BOOKMARK, KIND_HIGHLIGHT, ORIGIN_KOREADER,
};
use bookx_serve::syncplan::{self, Annot, Pairing, SideBook};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::core::devstyle::{self, Look};
use crate::core::library::{Book, Highlight};
use crate::device_sync::Locator;
use crate::highlight_goto::Hint;
use crate::i18n::{ntr, tr, trf};

/// What became of one KOReader annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// A library highlight stands for it.
    Linked,
    /// Deleted on the device (and here, or kept because the reader changed it here).
    Removed,
    /// A bookmark. One that was imported has its Bookx bookmark in [`Ledger::bookmarks`]; the
    /// entry is what remembers it (an older sync only remembered it, and imports it now).
    Bookmark,
}

/// One annotation received from a device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Incoming {
    pub book_id: u64,
    pub highlight: Option<String>,
    /// `KOReader (Kobo Libra)`.
    pub source: String,
    /// The note both sides had at the last sync.
    pub note: String,
    /// The look the device had at the last sync, so a colour changed on the device is taken but
    /// a look the reader chose in Bookx is not reset by an unchanged device record.
    #[serde(default)]
    pub device_look: Look,
    pub updated: i64,
    /// Server change number last handled.
    pub seq: u64,
    pub state: State,
    /// A copy of the quote, so an unplaced one can be shown and retried.
    pub text: String,
    /// The highlight was added by this sync (a deletion on the device may remove it again); a
    /// highlight the reader already had is only linked and never removed by a device.
    #[serde(default)]
    pub made_here: bool,
    /// Where the device had it (xpointers, chapter, page): used to open the book at the exact
    /// occurrence of a quote that is in the book more than once.
    #[serde(default)]
    pub hint: Hint,
}

/// One highlight of the library that was sent to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outgoing {
    pub fingerprint: String,
    pub updated: i64,
    pub deleted: bool,
}

/// The digest of a book file, remembered while the file does not change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Digest {
    pub file: String,
    pub size: u64,
    pub md5: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ledger {
    /// Highest server change number merged completely.
    pub seen: u64,
    /// `document/annotation id`
    pub incoming: BTreeMap<String, Incoming>,
    /// `document/highlight id`
    pub outgoing: BTreeMap<String, Outgoing>,
    /// document digest -> library book id
    pub docs: BTreeMap<String, u64>,
    pub digests: BTreeMap<u64, Digest>,
    /// `document/annotation id` -> the Bookx bookmark a device bookmark became. An entry stays
    /// when the reader deletes the bookmark here (it is the tombstone: it is never imported
    /// again) and when the device deletes it (nothing is ever deleted by a sync).
    pub bookmarks: BTreeMap<String, crate::bookmark_sync::Entry>,
    /// Unix seconds of the last merge.
    pub last_merge: i64,
}

fn key(doc: &str, id: &str) -> String {
    format!("{doc}/{id}")
}

impl Ledger {
    pub fn load(path: &Path) -> Ledger {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
    }

    /// Where the highlight came from, when it was made by this sync (`KOReader (Kobo Libra)`).
    pub fn source_of(&self, book_id: u64, highlight_id: &str) -> Option<&str> {
        self.incoming
            .values()
            .find(|i| {
                i.book_id == book_id
                    && i.state == State::Linked
                    && i.highlight.as_deref() == Some(highlight_id)
            })
            .map(|i| i.source.as_str())
    }
}

/// The ledger file inside the data folder.
pub fn ledger_path(data_dir: &Path) -> PathBuf {
    data_dir.join("annotation-sync.json")
}

/// What a merge did, for the status line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub added: usize,
    pub linked: usize,
    pub notes: usize,
    pub removed: usize,
    pub kept: usize,
    pub conflicts: usize,
    pub unplaced: usize,
    /// Bookmarks added to books.
    pub bookmarks: usize,
    /// Bookmarks that were already in the book at that place (linked, not added again).
    pub bookmarks_skipped: usize,
    /// Books of the device that are not in the library.
    pub unmatched: usize,
    /// Highlights and tombstones sent to the server.
    pub sent: usize,
    /// Highlights whose colour or style was corrected from the device.
    pub looks: usize,
}

impl Report {
    /// One short sentence for the sharing page.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.added + self.linked > 0 {
            parts.push(trf(
                "{} highlights from KOReader",
                &[&(self.added + self.linked)],
            ));
        }
        if self.bookmarks > 0 {
            parts.push(ntr(
                "{} bookmark from KOReader",
                "{} bookmarks from KOReader",
                self.bookmarks,
            ));
        }
        if self.bookmarks_skipped > 0 {
            parts.push(ntr(
                "{} bookmark already in your library",
                "{} bookmarks already in your library",
                self.bookmarks_skipped,
            ));
        }
        if self.notes > 0 {
            parts.push(trf("{} notes updated", &[&self.notes]));
        }
        if self.looks > 0 {
            parts.push(trf("{} colours updated", &[&self.looks]));
        }
        if self.removed > 0 {
            parts.push(trf("{} removed", &[&self.removed]));
        }
        if self.sent > 0 {
            parts.push(trf("{} sent to the device", &[&self.sent]));
        }
        if self.unplaced > 0 {
            parts.push(trf("{} not found in the book", &[&self.unplaced]));
        }
        if self.conflicts > 0 {
            parts.push(trf("{} kept as in your library", &[&self.conflicts]));
        }
        if self.unmatched > 0 {
            parts.push(trf("{} books not in your library", &[&self.unmatched]));
        }
        if parts.is_empty() {
            tr("Nothing new")
        } else {
            parts.join(", ")
        }
    }
}

/// KOReader's highlight colour as a stored colour name (`devstyle`: a palette colour, one the
/// palette lacks such as `cyan` or `olive` kept as written, any other word kept as written too).
/// Only an empty word is yellow.
pub fn color_in(name: &str) -> String {
    devstyle::koreader_color(name).unwrap_or_else(|| "yellow".into())
}

/// Bookx's colour as the KOReader name.
pub fn color_out(name: &str) -> String {
    devstyle::to_koreader_color(name)
}

/// KOReader's `drawer` as a stored highlight style (`lighten` is `background`).
pub fn style_in(drawer: &str) -> String {
    devstyle::canonical_style(drawer).unwrap_or_else(|| "background".into())
}

pub fn drawer_out(style: &str) -> String {
    devstyle::to_koreader_drawer(style)
}

/// The look a device annotation asks for. A part the device did not say is `None`; the words are
/// the ones Bookx stores (`lighten` becomes `background`, `olive`/`cyan` stay as written).
fn look_want(a: &Annotation) -> (Option<String>, Option<String>) {
    devstyle::look_of(
        (!a.color.trim().is_empty()).then_some(a.color.as_str()),
        (!a.drawer.trim().is_empty()).then_some(a.drawer.as_str()),
    )
}

fn same_style(stored: &str, other: &str) -> bool {
    devstyle::full_look("", stored).style == devstyle::full_look("", other).style
}

fn same_color(stored: &str, other: &str) -> bool {
    devstyle::full_look(stored, "").color == devstyle::full_look(other, "").color
}

/// The remembered look with the parts of `want` that the device said applied.
fn with_want(remembered: &Look, want: &(Option<String>, Option<String>)) -> Look {
    let mut look = remembered.clone();
    if let Some(c) = &want.0 {
        look.color = c.clone();
    }
    if let Some(s) = &want.1 {
        look.style = s.clone();
    }
    look
}

fn authors_of(s: &str) -> Vec<String> {
    s.split(['\n', ';'])
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect()
}

/// The 32 hex digits KOReader names a book by, from the file (the partial MD5 of KOSync).
pub fn digest_of_file(path: &Path) -> Option<String> {
    bookx_serve::kosync_client::partial_md5(path).ok()
}

/// Fills `ledger.docs` for the documents the server knows: by the book's file digest, else by
/// title and authors when exactly one book fits. Books are digested once per file state.
/// Returns the documents that match no book.
pub fn resolve_docs(
    root: &Path,
    books: &[Book],
    docs: &[(String, String, String, String)],
    ledger: &mut Ledger,
    digest: &dyn Fn(&Path) -> Option<String>,
) -> Vec<String> {
    let mut unmatched = Vec::new();
    let mut wanted: Vec<&str> = docs.iter().map(|d| d.1.as_str()).collect();
    wanted.sort();
    wanted.dedup();
    // digest the books only when there is a document that is not resolved yet
    let todo: Vec<&str> = wanted
        .iter()
        .copied()
        .filter(|d| !ledger.docs.contains_key(*d))
        .collect();
    if !todo.is_empty() {
        for b in books {
            let size = b.size;
            let fresh = ledger
                .digests
                .get(&b.id)
                .is_some_and(|d| d.file == b.file && d.size == size);
            if !fresh {
                if let Some(md5) = digest(&root.join(&b.file)) {
                    ledger.digests.insert(
                        b.id,
                        Digest {
                            file: b.file.clone(),
                            size,
                            md5,
                        },
                    );
                }
            }
        }
    }
    let side: Vec<SideBook> = books
        .iter()
        .map(|b| SideBook {
            id: b.id.to_string(),
            title: b.meta.title.clone(),
            authors: b.meta.authors.clone(),
            progress: 0.0,
            status: bookx_serve::reading::Status::Reading,
            last_read: None,
            notes: Vec::new(),
        })
        .collect();
    for (_, doc, title, authors) in docs {
        if ledger.docs.contains_key(doc) {
            if books.iter().any(|b| Some(&b.id) == ledger.docs.get(doc)) {
                continue;
            }
            ledger.docs.remove(doc);
        }
        let by_digest = ledger
            .digests
            .iter()
            .find(|(_, d)| d.md5.eq_ignore_ascii_case(doc))
            .map(|(id, _)| *id)
            .filter(|id| books.iter().any(|b| b.id == *id));
        let found = by_digest.or_else(|| {
            if title.trim().is_empty() {
                return None;
            }
            match syncplan::pair(&side, title, &authors_of(authors)) {
                Pairing::One(i) => Some(books[i].id),
                _ => None,
            }
        });
        match found {
            Some(id) => {
                ledger.docs.insert(doc.clone(), id);
            }
            None => unmatched.push(doc.clone()),
        }
    }
    unmatched.sort();
    unmatched.dedup();
    unmatched
}

fn when(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).single().unwrap_or_else(Utc::now)
}

fn same_quote(a: &str, b: &str) -> bool {
    let x = |t: &str| Annot {
        text: t.to_string(),
        note: String::new(),
        when: None,
    };
    syncplan::same_highlight(&x(a), &x(b))
}

/// A device bookmark as the place resolver reads it.
fn mark_of(a: &Annotation) -> crate::bookmark_sync::DeviceMark {
    crate::bookmark_sync::DeviceMark {
        id: a.id.clone(),
        note: a.note.trim().to_string(),
        chapter: a.chapter.trim().to_string(),
        pos: a.pos0.trim().to_string(),
        page: a.page,
        percentage: a.percentage,
        href: None,
        chapter_progress: None,
        created: a.created,
    }
}

fn source_name(device: &str) -> String {
    let d = device.trim();
    if d.is_empty() {
        "KOReader".to_string()
    } else {
        format!("KOReader ({d})")
    }
}

/// Merges what devices sent into the library. Idempotent: running it again with the same
/// changes does nothing. `changes` may hold several versions of nothing: each record is the
/// newest the server has.
pub fn merge_incoming(
    books: &mut [Book],
    changes: &[Change],
    ledger: &mut Ledger,
    locator: &dyn Locator,
    now: DateTime<Utc>,
) -> Report {
    let mut rep = Report::default();
    let mut unmatched = std::collections::BTreeSet::new();
    // longest text first: a passage claims its own library highlight before a shorter quote that is
    // part of it can (the server's order says nothing about which of the two the library has)
    let mut order: Vec<&Change> = changes.iter().collect();
    order.sort_by_key(|c| std::cmp::Reverse(syncplan::norm(&c.record.annotation.text).len()));
    for c in order {
        let r = &c.record;
        if r.origin != ORIGIN_KOREADER {
            continue;
        }
        let a = &r.annotation;
        let Some(&book_id) = ledger.docs.get(&c.document) else {
            unmatched.insert(c.document.clone());
            continue;
        };
        let Some(book) = books.iter_mut().find(|b| b.id == book_id) else {
            unmatched.insert(c.document.clone());
            continue;
        };
        let k = key(&c.document, &a.id);
        let prev = ledger.incoming.get(&k).cloned();
        let is_mark = a.kind == KIND_BOOKMARK;
        if let Some(p) = &prev {
            // handled before; an older sync only remembered a bookmark: it is imported now
            let unimported = is_mark && !a.deleted && !ledger.bookmarks.contains_key(&k);
            if p.seq >= r.seq && !unimported {
                continue;
            }
        }
        let source = source_name(&r.device);
        let entry = if is_mark {
            // a bookmark the device deleted is not removed here, and one never seen is not added
            if !a.deleted {
                let m = mark_of(a);
                let place = locator.locate_place(book_id, &m);
                match crate::bookmark_sync::import(
                    book,
                    &mut ledger.bookmarks,
                    &k,
                    &m,
                    &place,
                    &source,
                    now,
                ) {
                    crate::bookmark_sync::Outcome::Added => rep.bookmarks += 1,
                    crate::bookmark_sync::Outcome::Linked => rep.bookmarks_skipped += 1,
                    crate::bookmark_sync::Outcome::Known => {}
                }
            }
            Incoming {
                book_id,
                highlight: ledger.bookmarks.get(&k).map(|e| e.bookmark.clone()),
                source,
                note: a.note.clone(),
                device_look: Look::default(),
                updated: a.updated,
                seq: r.seq,
                state: State::Bookmark,
                text: String::new(),
                made_here: false,
                hint: Hint::default(),
            }
        } else if a.deleted {
            let mut e = prev.clone().unwrap_or(Incoming {
                book_id,
                highlight: None,
                source,
                note: String::new(),
                device_look: Look::default(),
                updated: a.updated,
                seq: r.seq,
                state: State::Removed,
                text: String::new(),
                made_here: false,
                hint: Hint::default(),
            });
            if let (State::Linked, Some(hid)) = (e.state, e.highlight.clone()) {
                if a.updated >= e.updated && e.made_here {
                    match book.highlights.iter().position(|h| h.id == hid) {
                        Some(i) if book.highlights[i].note == e.note => {
                            book.highlights.remove(i);
                            rep.removed += 1;
                        }
                        Some(_) => rep.kept += 1,
                        None => {}
                    }
                }
            }
            e.state = State::Removed;
            e.seq = r.seq;
            e.updated = a.updated.max(e.updated);
            e
        } else {
            // highlights another annotation of the device already stands for
            let claimed: std::collections::HashSet<String> = ledger
                .incoming
                .iter()
                .filter(|(other, e)| {
                    **other != k && e.book_id == book_id && e.state == State::Linked
                })
                .filter_map(|(_, e)| e.highlight.clone())
                .collect();
            live(
                book, &mut rep, prev, a, &r.device, r.seq, locator, now, &claimed,
            )
        };
        ledger.incoming.insert(k, entry);
    }
    rep.unmatched = unmatched.len();
    rep
}

/// Repairs highlights that an earlier sync linked with Bookx's default yellow although the device
/// had sent a colour. This runs on every merge, including a forced one where the server change
/// number did not move (the owner's report: every highlight was still yellow). It is idempotent:
/// a highlight that is not Bookx's default look is never overwritten, and the device's word is
/// remembered so a later unchanged record cannot reset a look chosen in Bookx.
pub fn repair_looks(books: &mut [Book], changes: &[Change], ledger: &mut Ledger) -> usize {
    let mut fixed = 0;
    for c in changes {
        if c.record.origin != ORIGIN_KOREADER {
            continue;
        }
        let Some(&book_id) = ledger.docs.get(&c.document) else {
            continue;
        };
        let Some(book) = books.iter_mut().find(|b| b.id == book_id) else {
            continue;
        };
        let k = key(&c.document, &c.record.annotation.id);
        let Some(entry) = ledger.incoming.get_mut(&k) else {
            continue;
        };
        if entry.state != State::Linked {
            continue;
        }
        // highlights imported before the device's place was kept get it now
        let hint = hint_of(&c.record.annotation);
        if entry.hint != hint {
            entry.hint = hint;
        }
        let Some(highlight_id) = entry.highlight.clone() else {
            continue;
        };
        let Some(h) = book.highlights.iter_mut().find(|h| h.id == highlight_id) else {
            continue;
        };
        let want = look_want(&c.record.annotation);
        if let Some(color) = &want.0 {
            entry.device_look.color = color.clone();
        }
        if let Some(style) = &want.1 {
            entry.device_look.style = style.clone();
        }
        if !devstyle::is_default_look(&h.color, &h.style) {
            continue;
        }
        let mut changed = false;
        if let Some(color) = &want.0 {
            if !same_color(&h.color, color) {
                h.color = color.clone();
                changed = true;
            }
        }
        if let Some(style) = &want.1 {
            if !same_style(&h.style, style) {
                h.style = style.clone();
                changed = true;
            }
        }
        if changed {
            fixed += 1;
        }
    }
    fixed
}

#[allow(clippy::too_many_arguments)]
fn live(
    book: &mut Book,
    rep: &mut Report,
    prev: Option<Incoming>,
    a: &Annotation,
    device: &str,
    seq: u64,
    locator: &dyn Locator,
    now: DateTime<Utc>,
    claimed: &std::collections::HashSet<String>,
) -> Incoming {
    let source = source_name(device);
    let book_id = book.id;
    let mut e = prev.clone().unwrap_or(Incoming {
        book_id,
        highlight: None,
        source,
        note: String::new(),
        device_look: Look::default(),
        updated: 0,
        seq,
        state: State::Removed,
        text: a.text.clone(),
        made_here: false,
        hint: Hint::default(),
    });
    e.seq = seq;
    e.text = a.text.clone();
    e.hint = hint_of(a);
    let want = look_want(a);
    let device_look = with_want(&e.device_look, &want);
    let look_changed = device_look != e.device_look;
    // the highlight this stands for, if it is still there
    let linked = e
        .highlight
        .as_ref()
        .filter(|_| e.state == State::Linked)
        .and_then(|id| book.highlights.iter().position(|h| h.id == *id));
    if let Some(i) = linked {
        let h = &mut book.highlights[i];
        // A colour changed on the device wins. A highlight imported before colours were read has
        // Bookx's default look; the reader never chose it, so the device's look repairs it even
        // when the device did not bump its timestamp. An unchanged device record never resets a
        // look the reader chose in Bookx.
        let repair =
            devstyle::is_default_look(&h.color, &h.style) && (want.0.is_some() || want.1.is_some());
        let apply_look = look_changed || repair;
        if a.updated > e.updated {
            if h.note == e.note {
                if h.note != a.note {
                    h.note = a.note.clone();
                    rep.notes += 1;
                }
                e.note = a.note.clone();
            } else if h.note != a.note {
                rep.conflicts += 1;
            }
            e.updated = a.updated;
        }
        if apply_look {
            if let Some(c) = &want.0 {
                if !same_color(&h.color, c) {
                    h.color = c.clone();
                    rep.looks += 1;
                }
            }
            if let Some(s) = &want.1 {
                if !same_style(&h.style, s) {
                    h.style = s.clone();
                    rep.looks += 1;
                }
            }
        }
        e.device_look = device_look;
        return e;
    }
    // not linked (or the linked highlight was deleted in the library: the reader's choice stays)
    if e.state == State::Linked {
        // the reader deleted the highlight here: it stays deleted until the device changes it again
        e.state = State::Removed;
    }
    if e.state == State::Removed && prev.is_some() && a.updated <= e.updated {
        return e;
    }
    // the highlight with these very words first (a short quote that is part of a longer highlight is
    // not that highlight), and never one that another annotation of the device already stands for
    let words = syncplan::norm(&a.text);
    let open = |h: &Highlight| {
        !crate::pdf::is_pdf(h) && !claimed.contains(&h.id) && same_quote(&h.text, &a.text)
    };
    let found = book
        .highlights
        .iter()
        .position(|h| open(h) && syncplan::norm(&h.text) == words)
        .or_else(|| book.highlights.iter().position(open));
    if let Some(h) = found.map(|i| &mut book.highlights[i]) {
        if h.note.trim().is_empty() && !a.note.trim().is_empty() {
            h.note = a.note.trim().to_string();
            rep.notes += 1;
        }
        // a highlight imported before colours were read has Bookx's default look: the device's look
        // is the true one (the same rule as the USB sync)
        if crate::core::devstyle::is_default_look(&h.color, &h.style) {
            if !a.color.is_empty() {
                h.color = color_in(&a.color);
            }
            if !a.drawer.is_empty() {
                h.style = style_in(&a.drawer);
            }
        }
        e.highlight = Some(h.id.clone());
        e.state = State::Linked;
        e.made_here = false;
        e.note = h.note.clone();
        e.device_look = device_look;
        e.updated = a.updated;
        rep.linked += 1;
        return e;
    }
    let chapter = Some(a.chapter.as_str()).filter(|c| !c.trim().is_empty());
    let mut spot = locator.locate_hinted(book_id, &a.text, chapter, &e.hint);
    if let Some(first) = spot {
        // the words may be highlighted at several places: a second highlight goes to another one
        let taken: Vec<(usize, i32, i32)> = book
            .highlights
            .iter()
            .map(|h| (h.spine, h.start, h.end))
            .collect();
        if taken.contains(&first) {
            spot = locator
                .locate_other(book_id, &a.text, chapter, &taken)
                .or(spot);
        }
    }
    match spot {
        Some((spine, start, end)) if start >= 0 && end > start => {
            let id = crate::annotation_ids::incoming_id(book, &a.text);
            book.highlights.push(Highlight {
                id: id.clone(),
                spine,
                start,
                end,
                color: color_in(&a.color),
                style: style_in(&a.drawer),
                note: a.note.trim().to_string(),
                text: a.text.trim().to_string(),
                created: if a.created > 0 { when(a.created) } else { now },
            });
            e.highlight = Some(id);
            e.state = State::Linked;
            e.made_here = true;
            e.note = a.note.trim().to_string();
            e.device_look = device_look;
            e.updated = a.updated;
            rep.added += 1;
        }
        _ => {
            // not in the book's text: kept as a note that is painted nowhere
            let id = crate::annotation_ids::incoming_id(book, &a.text);
            let (spine, start, end) = crate::core::anchor::unplaced(0);
            book.highlights.push(Highlight {
                id: id.clone(),
                spine,
                start,
                end,
                color: color_in(&a.color),
                style: style_in(&a.drawer),
                note: a.note.trim().to_string(),
                text: a.text.trim().to_string(),
                created: if a.created > 0 { when(a.created) } else { now },
            });
            e.highlight = Some(id);
            e.state = State::Linked;
            e.made_here = true;
            e.note = a.note.trim().to_string();
            e.device_look = device_look;
            e.updated = a.updated;
            rep.unplaced += 1;
        }
    }
    e
}

/// What identifies the content of a highlight, to notice a change. The look is compared in its
/// canonical words, so `""` and `background`, or `Grey` and `gray`, are not a change that goes out
/// to the device as an update.
fn fingerprint(h: &Highlight) -> String {
    let look = devstyle::full_look(&h.color, &h.style);
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        h.text, h.note, look.color, look.style
    )
}

/// The annotations to send to the server for each document, and the ledger entries to keep once
/// they were accepted ([`commit_outgoing`]). Only books a device has synced are included; a
/// highlight that came from KOReader, a PDF or unanchored one is not sent.
pub fn outgoing(
    books: &[Book],
    ledger: &Ledger,
    now: i64,
) -> Vec<(String, Vec<Annotation>, Vec<(String, Outgoing)>)> {
    let mut out = Vec::new();
    for (doc, book_id) in &ledger.docs {
        let Some(book) = books.iter().find(|b| b.id == *book_id) else {
            continue;
        };
        let from_device: std::collections::HashSet<&str> = ledger
            .incoming
            .iter()
            .filter(|(k, i)| k.starts_with(&format!("{doc}/")) && i.state == State::Linked)
            .filter_map(|(_, i)| i.highlight.as_deref())
            .collect();
        let mut list = Vec::new();
        let mut entries = Vec::new();
        let mut present = std::collections::HashSet::new();
        for h in &book.highlights {
            if crate::pdf::is_pdf(h) || h.start < 0 || h.text.trim().is_empty() {
                continue;
            }
            if from_device.contains(h.id.as_str()) || !sendable_id(&h.id) {
                continue;
            }
            present.insert(h.id.clone());
            let fp = fingerprint(h);
            let k = key(doc, &h.id);
            let prev = ledger.outgoing.get(&k);
            if prev.is_some_and(|p| p.fingerprint == fp && !p.deleted) {
                continue;
            }
            let updated = if prev.is_some() {
                now
            } else {
                h.created.timestamp().min(now)
            };
            list.push(Annotation {
                id: h.id.clone(),
                kind: KIND_HIGHLIGHT.into(),
                text: h.text.clone(),
                note: h.note.clone(),
                color: color_out(&h.color),
                drawer: drawer_out(&h.style),
                created: h.created.timestamp().min(now),
                updated,
                ..Annotation::default()
            });
            entries.push((
                k,
                Outgoing {
                    fingerprint: fp,
                    updated,
                    deleted: false,
                },
            ));
        }
        let prefix = format!("{doc}/");
        for (k, o) in ledger.outgoing.range(prefix.clone()..) {
            let Some(id) = k.strip_prefix(&prefix) else {
                break;
            };
            if o.deleted || present.contains(id) {
                continue;
            }
            list.push(Annotation {
                id: id.to_string(),
                kind: KIND_HIGHLIGHT.into(),
                deleted: true,
                updated: now,
                created: 0,
                ..Annotation::default()
            });
            entries.push((
                k.clone(),
                Outgoing {
                    fingerprint: o.fingerprint.clone(),
                    updated: now,
                    deleted: true,
                },
            ));
        }
        if !list.is_empty() {
            out.push((doc.clone(), list, entries));
        }
    }
    out
}

fn sendable_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= 128
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '@' | '+'))
}

/// Records what the server accepted.
pub fn commit_outgoing(ledger: &mut Ledger, entries: Vec<(String, Outgoing)>) {
    for (k, o) in entries {
        ledger.outgoing.insert(k, o);
    }
}

/// Where the device had an annotation: the hints that are worth keeping.
pub fn hint_of(a: &Annotation) -> Hint {
    Hint {
        chapter: a.chapter.trim().to_string(),
        pos0: a.pos0.trim().to_string(),
        pos1: a.pos1.trim().to_string(),
        page: a.page,
    }
}

/// The device's place for the highlights that a sync added, by `(book id, highlight id)`.
pub fn hints(ledger: &Ledger) -> HashMap<(u64, String), Hint> {
    ledger
        .incoming
        .values()
        .filter(|i| i.state == State::Linked && i.made_here && !i.hint.is_empty())
        .filter_map(|i| {
            i.highlight
                .clone()
                .map(|h| ((i.book_id, h), i.hint.clone()))
        })
        .collect()
}

/// Which book (id) each library highlight id belongs to, for the notes list: nothing to do with
/// the server, kept here so the Notes view can show the source.
pub fn sources(ledger: &Ledger) -> HashMap<(u64, String), String> {
    let mut map: HashMap<(u64, String), String> = ledger
        .incoming
        .values()
        .filter(|i| i.state == State::Linked)
        .filter_map(|i| {
            i.highlight
                .clone()
                .map(|h| ((i.book_id, h), i.source.clone()))
        })
        .collect();
    // the bookmarks the devices brought (their ids are not highlight ids)
    map.extend(
        ledger
            .bookmarks
            .values()
            .map(|e| ((e.book_id, e.bookmark.clone()), e.source.clone())),
    );
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_serve::annotation_sync::{Record, ORIGIN_BOOKX};

    struct Fixed(Vec<(&'static str, (usize, i32, i32))>);

    impl Locator for Fixed {
        fn locate(&self, _: u64, text: &str) -> Option<(usize, i32, i32)> {
            self.0.iter().find(|(t, _)| *t == text).map(|(_, p)| *p)
        }
    }

    const DOC: &str = "0123456789abcdef0123456789abcdef";

    fn book() -> Book {
        serde_json::from_value(serde_json::json!({
            "id": 1, "file": "a.epub", "meta": {"title": "Dune", "title_sort": "", "authors": ["Frank Herbert"], "author_sort": "", "publisher": "", "language": "", "description": "", "tags": [], "series": "", "series_index": 0.0, "identifiers": [], "published": "", "rights": "", "rating": 0},
            "added": Utc::now(), "modified": Utc::now(), "size": 1
        }))
        .unwrap()
    }

    fn change(id: &str, text: &str, note: &str, updated: i64, seq: u64) -> Change {
        Change {
            user: "u".into(),
            document: DOC.into(),
            title: "Dune".into(),
            authors: "Frank Herbert".into(),
            record: Record {
                annotation: Annotation {
                    id: id.into(),
                    kind: KIND_HIGHLIGHT.into(),
                    text: text.into(),
                    note: note.into(),
                    color: "green".into(),
                    drawer: "underscore".into(),
                    created: 1_700_000_000,
                    updated,
                    ..Annotation::default()
                },
                origin: ORIGIN_KOREADER.into(),
                device: "Kobo Libra".into(),
                device_id: "d".into(),
                seq,
                stored: updated,
            },
        }
    }

    fn deleted(id: &str, updated: i64, seq: u64) -> Change {
        let mut c = change(id, "", "", updated, seq);
        c.record.annotation.deleted = true;
        c
    }

    fn setup() -> (Vec<Book>, Ledger, Fixed) {
        let mut l = Ledger::default();
        l.docs.insert(DOC.into(), 1);
        (
            vec![book()],
            l,
            Fixed(vec![
                ("Fear is the mind-killer.", (2, 10, 34)),
                ("A second long quotation.", (3, 5, 29)),
            ]),
        )
    }

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0).unwrap()
    }

    #[test]
    fn a_found_quote_is_added_with_its_provenance() {
        let (mut books, mut l, loc) = setup();
        let r = merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "my note", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(r.added, 1);
        let h = &books[0].highlights[0];
        assert_eq!((h.spine, h.start, h.end), (2, 10, 34));
        assert_eq!(h.note, "my note");
        assert_eq!(h.color, "green");
        assert_eq!(h.style, "underline");
        assert_eq!(l.source_of(1, &h.id), Some("KOReader (Kobo Libra)"));
        assert_eq!(sources(&l).len(), 1);
    }

    /// A locator that places a quote where the device's xpointer says (`DocFragment[N]` is chapter N-1).
    struct ByPointer;

    impl Locator for ByPointer {
        fn locate(&self, _: u64, _: &str) -> Option<(usize, i32, i32)> {
            Some((0, 0, 5))
        }

        fn locate_hinted(
            &self,
            _: u64,
            _: &str,
            _: Option<&str>,
            hint: &Hint,
        ) -> Option<(usize, i32, i32)> {
            let x = crate::highlight_goto::parse_xpointer(&hint.pos0)?;
            Some((x.fragment - 1, 7, 12))
        }
    }

    fn with_pointer(mut c: Change, pos0: &str, chapter: &str) -> Change {
        c.record.annotation.pos0 = pos0.into();
        c.record.annotation.chapter = chapter.into();
        c.record.annotation.page = Some(42);
        c
    }

    #[test]
    fn where_the_device_had_a_highlight_is_remembered_and_places_it() {
        let (mut books, mut l, _) = setup();
        let c = with_pointer(
            change("a", "Fear is the mind-killer.", "", 100, 1),
            "/body/DocFragment[12]/body/div/p[3]/text().5",
            "Litany",
        );
        merge_incoming(&mut books, &[c], &mut l, &ByPointer, now());
        let h = &books[0].highlights[0];
        assert_eq!(
            (h.spine, h.start),
            (11, 7),
            "placed through the device's xpointer"
        );
        let hint = hints(&l).remove(&(1, h.id.clone())).expect("a hint");
        assert_eq!(hint.chapter, "Litany");
        assert_eq!(hint.pos0, "/body/DocFragment[12]/body/div/p[3]/text().5");
        assert_eq!(hint.page, Some(42));
    }

    #[test]
    fn a_highlight_the_library_already_had_has_no_hint_to_override_its_place() {
        let (mut books, mut l, loc) = setup();
        books[0].highlights.push(Highlight {
            id: "own".into(),
            spine: 2,
            start: 10,
            end: 34,
            color: "yellow".into(),
            style: String::new(),
            note: String::new(),
            text: "Fear is the mind-killer.".into(),
            created: now(),
        });
        let c = with_pointer(
            change("a", "Fear is the mind-killer.", "", 100, 1),
            "/body/DocFragment[9]/body/p/text().0",
            "",
        );
        merge_incoming(&mut books, &[c], &mut l, &loc, now());
        assert!(
            hints(&l).is_empty(),
            "only a highlight the sync made is placed by the device"
        );
    }

    #[test]
    fn highlights_synced_before_places_were_kept_get_them_on_the_next_merge() {
        let (mut books, mut l, loc) = setup();
        let plain = change("a", "Fear is the mind-killer.", "", 100, 1);
        merge_incoming(
            &mut books,
            std::slice::from_ref(&plain),
            &mut l,
            &loc,
            now(),
        );
        assert!(hints(&l).is_empty());
        let c = with_pointer(plain, "/body/DocFragment[3]/body/p/text().0", "");
        repair_looks(&mut books, &[c], &mut l);
        assert_eq!(hints(&l).len(), 1);
    }

    #[test]
    fn a_repeat_changes_nothing() {
        let (mut books, mut l, loc) = setup();
        let c = [change("a", "Fear is the mind-killer.", "n", 100, 1)];
        merge_incoming(&mut books, &c, &mut l, &loc, now());
        let before = (books[0].highlights.clone(), l.clone());
        let r = merge_incoming(&mut books, &c, &mut l, &loc, now());
        assert_eq!(r, Report::default(), "{r:?}");
        assert_eq!((books[0].highlights.clone(), l), before);
    }

    #[test]
    fn a_quote_that_is_not_in_the_book_is_kept_unanchored_never_placed() {
        let (mut books, mut l, loc) = setup();
        let c = [change(
            "a",
            "Words the book does not contain.",
            "my thought",
            100,
            1,
        )];
        let r = merge_incoming(&mut books, &c, &mut l, &loc, now());
        assert_eq!((r.added, r.unplaced), (0, 1));
        let h = &books[0].highlights[0];
        assert!(crate::core::anchor::is_unanchored(h), "painted nowhere");
        assert_eq!(h.note, "my thought");
        assert_eq!(h.text, "Words the book does not contain.");
        // a repeat does not add it again
        let r = merge_incoming(&mut books, &c, &mut l, &loc, now());
        assert_eq!(r, Report::default());
        assert_eq!(books[0].highlights.len(), 1);
        // and it is never sent back to the device
        assert!(outgoing(&books, &l, 1000).is_empty());
    }

    fn mine(id: &str, text: &str, start: i32) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 2,
            start,
            end: start + text.chars().count() as i32,
            color: "yellow".into(),
            style: String::new(),
            note: String::new(),
            text: text.into(),
            created: now(),
        }
    }

    /// Found on the owner's Kobo (wireless replay of its real sidecar): the short annotation "Event storming,"
    /// was linked to the long highlight that contains those words, the library's own short highlight stayed
    /// unlinked and went back to the device as a Bookx note (23 times in *Building Microservices*).
    #[test]
    fn a_short_quote_and_the_longer_highlight_around_it_link_to_their_own() {
        let (mut books, mut l, loc) = setup();
        let long = "Event storming brings together technical and nontechnical stakeholders.";
        books[0].highlights = vec![
            mine("long", long, 100),
            mine("short", "Event storming,", 400),
        ];
        let c = [
            change("s", "Event storming,", "", 100, 1),
            change("l", long, "", 100, 2),
        ];
        let r = merge_incoming(&mut books, &c, &mut l, &loc, now());
        assert_eq!((r.added, r.linked), (0, 2), "{r:?}");
        assert_eq!(
            l.incoming[&key(DOC, "s")].highlight.as_deref(),
            Some("short")
        );
        assert_eq!(
            l.incoming[&key(DOC, "l")].highlight.as_deref(),
            Some("long")
        );
        assert!(
            outgoing(&books, &l, 1000)
                .iter()
                .all(|(_, list, _)| list.is_empty()),
            "everything the library has came from the device"
        );
    }

    /// Wireless replay of the owner's real sidecar: 123 highlights imported earlier have Bookx's default
    /// yellow, the device says cyan, purple, ... and KOReader's cyan was turned into blue.
    #[test]
    fn a_linked_highlight_with_the_default_look_takes_the_devices_look_and_other_looks_stay() {
        let (mut books, mut l, loc) = setup();
        books[0].highlights = vec![
            mine("a", "Fear is the mind-killer.", 10),
            mine("b", "A second long quotation.", 50),
        ];
        books[0].highlights[1].color = "pink".into();
        let mut c1 = change("1", "Fear is the mind-killer.", "", 100, 1);
        c1.record.annotation.color = "cyan".into();
        c1.record.annotation.drawer = "underscore".into();
        let c2 = change("2", "A second long quotation.", "", 100, 2);
        merge_incoming(&mut books, &[c1, c2], &mut l, &loc, now());
        assert_eq!(
            (
                books[0].highlights[0].color.as_str(),
                books[0].highlights[0].style.as_str()
            ),
            ("cyan", "underline")
        );
        assert_eq!(
            books[0].highlights[1].color, "pink",
            "a look the reader chose stays"
        );
    }

    #[test]
    fn the_same_words_in_two_places_are_two_highlights() {
        struct Two;
        impl Locator for Two {
            fn locate(&self, _: u64, _: &str) -> Option<(usize, i32, i32)> {
                Some((2, 10, 26))
            }
            fn locate_other(
                &self,
                _: u64,
                _: &str,
                _: Option<&str>,
                taken: &[(usize, i32, i32)],
            ) -> Option<(usize, i32, i32)> {
                [(2, 10, 26), (2, 900, 916)]
                    .into_iter()
                    .find(|p| !taken.contains(p))
            }
        }
        let (mut books, mut l, _) = setup();
        let c = [
            change("a", "Request-response", "", 100, 1),
            change("b", "Request-response", "", 100, 2),
        ];
        let r = merge_incoming(&mut books, &c, &mut l, &Two, now());
        assert_eq!(r.added, 2);
        let mut at: Vec<_> = books[0].highlights.iter().map(|h| h.start).collect();
        at.sort();
        assert_eq!(at, [10, 900]);
    }

    #[test]
    fn the_same_quote_is_linked_not_duplicated() {
        let (mut books, mut l, loc) = setup();
        let mut h = Highlight {
            id: "mine".into(),
            spine: 2,
            start: 10,
            end: 34,
            color: "yellow".into(),
            style: String::new(),
            note: String::new(),
            text: "Fear is the mind-killer".into(),
            created: now(),
        };
        h.note = String::new();
        books[0].highlights.push(h);
        let r = merge_incoming(
            &mut books,
            &[change(
                "a",
                "fear is the mind-killer.",
                "from device",
                100,
                1,
            )],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!((r.added, r.linked, r.notes), (0, 1, 1));
        assert_eq!(books[0].highlights.len(), 1);
        assert_eq!(books[0].highlights[0].note, "from device");
        // an existing note is never overwritten by linking
        let (mut books, mut l, loc) = setup();
        let mut h = books_hl("mine", "Fear is the mind-killer.");
        h.note = "keep me".into();
        books[0].highlights.push(h);
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "other", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(books[0].highlights[0].note, "keep me");
    }

    fn books_hl(id: &str, text: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 2,
            start: 10,
            end: 34,
            color: "yellow".into(),
            style: String::new(),
            note: String::new(),
            text: text.into(),
            created: now(),
        }
    }

    #[test]
    fn a_newer_note_replaces_an_untouched_one_but_not_an_edited_one() {
        let (mut books, mut l, loc) = setup();
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "v1", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        let r = merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "v2", 200, 2)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(r.notes, 1);
        assert_eq!(books[0].highlights[0].note, "v2");
        // the reader edits it here, then the device edits it again
        books[0].highlights[0].note = "mine".into();
        let r = merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "v3", 300, 3)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!((r.notes, r.conflicts), (0, 1));
        assert_eq!(books[0].highlights[0].note, "mine");
    }

    #[test]
    fn a_stale_version_is_ignored() {
        let (mut books, mut l, loc) = setup();
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "new", 200, 2)],
            &mut l,
            &loc,
            now(),
        );
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "old", 100, 3)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(books[0].highlights[0].note, "new");
    }

    #[test]
    fn a_deletion_removes_only_what_the_sync_made_and_nobody_edited() {
        let (mut books, mut l, loc) = setup();
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        let r = merge_incoming(&mut books, &[deleted("a", 200, 2)], &mut l, &loc, now());
        assert_eq!(r.removed, 1);
        assert!(books[0].highlights.is_empty());

        // edited here: stays
        let (mut books, mut l, loc) = setup();
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        books[0].highlights[0].note = "I wrote this".into();
        let r = merge_incoming(&mut books, &[deleted("a", 200, 2)], &mut l, &loc, now());
        assert_eq!((r.removed, r.kept), (0, 1));
        assert_eq!(books[0].highlights.len(), 1);

        // a highlight the reader made in Bookx is only linked and never removed by a device
        let (mut books, mut l, loc) = setup();
        books[0]
            .highlights
            .push(books_hl("mine", "Fear is the mind-killer."));
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        let r = merge_incoming(&mut books, &[deleted("a", 200, 2)], &mut l, &loc, now());
        assert_eq!(r.removed, 0);
        assert_eq!(books[0].highlights.len(), 1);
    }

    #[test]
    fn a_deletion_older_than_the_last_change_is_ignored() {
        let (mut books, mut l, loc) = setup();
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "", 300, 3)],
            &mut l,
            &loc,
            now(),
        );
        let r = merge_incoming(&mut books, &[deleted("a", 200, 4)], &mut l, &loc, now());
        assert_eq!(r.removed, 0);
        assert_eq!(books[0].highlights.len(), 1);
    }

    #[test]
    fn a_highlight_deleted_in_the_library_does_not_come_back() {
        let (mut books, mut l, loc) = setup();
        let c = change("a", "Fear is the mind-killer.", "", 100, 1);
        merge_incoming(&mut books, std::slice::from_ref(&c), &mut l, &loc, now());
        books[0].highlights.clear();
        let mut again = c;
        again.record.seq = 2;
        merge_incoming(&mut books, &[again], &mut l, &loc, now());
        assert!(books[0].highlights.is_empty());
        // a real newer edit on the device may bring it back
        merge_incoming(
            &mut books,
            &[change(
                "a",
                "Fear is the mind-killer.",
                "new thought",
                500,
                3,
            )],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(books[0].highlights.len(), 1);
    }

    /// A locator that gives every device bookmark the same exact place (chapter 3, 40 % in).
    struct Marks(crate::bookmark_sync::Place);

    impl Locator for Marks {
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

    fn exact(spine: usize, frac: f64) -> Marks {
        Marks(crate::bookmark_sync::Place {
            spine,
            frac,
            how: crate::bookmark_sync::How::Xpointer,
            exact: true,
        })
    }

    /// A locator that puts a bookmark in chapter 3 at its page number in a hundred.
    struct ByPage;

    impl Locator for ByPage {
        fn locate(&self, _: u64, _: &str) -> Option<(usize, i32, i32)> {
            None
        }

        fn locate_place(
            &self,
            _: u64,
            m: &crate::bookmark_sync::DeviceMark,
        ) -> crate::bookmark_sync::Place {
            crate::bookmark_sync::Place {
                spine: 2,
                frac: m.page.unwrap_or(0) as f64 / 100.0,
                how: crate::bookmark_sync::How::Xpointer,
                exact: true,
            }
        }
    }

    fn mark_change(id: &str, page: i64, updated: i64, seq: u64) -> Change {
        let mut c = change(id, "", "", updated, seq);
        let a = &mut c.record.annotation;
        a.kind = KIND_BOOKMARK.into();
        a.page = Some(page);
        a.color.clear();
        a.drawer.clear();
        c
    }

    #[test]
    fn a_synced_bookmark_becomes_a_bookx_bookmark_with_its_place_and_origin() {
        let (mut books, mut l, _) = setup();
        let mut c = mark_change("b", 7, 100, 1);
        c.record.annotation.chapter = "Litany".into();
        let r = merge_incoming(&mut books, &[c], &mut l, &exact(2, 0.4), now());
        assert_eq!((r.bookmarks, r.bookmarks_skipped), (1, 0));
        assert!(books[0].highlights.is_empty(), "a bookmark is no highlight");
        let b = &books[0].bookmarks[0];
        assert_eq!((b.spine, b.frac, b.label.as_str()), (2, 0.4, "Page 7"));
        assert_eq!(b.created.timestamp(), 1_700_000_000, "the device's date");
        assert_eq!(
            sources(&l).get(&(1, b.id.clone())).map(String::as_str),
            Some("KOReader (Kobo Libra)")
        );
        let e = &l.bookmarks[&key(DOC, "b")];
        assert!(!e.approx);
        assert_eq!(e.hint.chapter, "Litany");
        assert!(
            r.text().contains("1 bookmark from KOReader"),
            "{}",
            r.text()
        );
    }

    #[test]
    fn the_note_on_a_device_bookmark_is_what_the_bookmark_says() {
        let (mut books, mut l, _) = setup();
        let mut c = mark_change("b", 7, 100, 1);
        c.record.annotation.note = "where I stopped".into();
        merge_incoming(&mut books, &[c], &mut l, &exact(2, 0.4), now());
        assert_eq!(books[0].bookmarks[0].label, "where I stopped");
    }

    #[test]
    fn a_repeat_of_the_same_bookmarks_changes_nothing() {
        let (mut books, mut l, _) = setup();
        let c = [mark_change("b", 7, 100, 1)];
        merge_incoming(&mut books, &c, &mut l, &exact(2, 0.4), now());
        let before = (books[0].bookmarks.clone(), l.clone());
        let r = merge_incoming(&mut books, &c, &mut l, &exact(2, 0.4), now());
        assert_eq!(r, Report::default());
        assert_eq!((books[0].bookmarks.clone(), l.clone()), before);
        // even a newer version of it is the same bookmark
        let r = merge_incoming(
            &mut books,
            &[mark_change("b", 7, 900, 5)],
            &mut l,
            &exact(2, 0.4),
            now(),
        );
        assert_eq!(r, Report::default());
        assert_eq!(books[0].bookmarks.len(), 1);
    }

    #[test]
    fn a_bookmark_the_book_has_at_that_place_is_not_added_again_and_stays_as_it_is() {
        let (mut books, mut l, _) = setup();
        books[0].bookmarks.push(crate::core::library::Bookmark {
            id: "mine".into(),
            spine: 2,
            frac: 0.405,
            label: "my label".into(),
            created: now(),
        });
        let r = merge_incoming(
            &mut books,
            &[mark_change("b", 7, 100, 1)],
            &mut l,
            &exact(2, 0.4),
            now(),
        );
        assert_eq!((r.bookmarks, r.bookmarks_skipped), (0, 1));
        assert_eq!(books[0].bookmarks.len(), 1);
        assert_eq!(books[0].bookmarks[0].label, "my label");
        assert_eq!(books[0].bookmarks[0].frac, 0.405);
        assert!(r.text().contains("1 bookmark already in your library"));
    }

    #[test]
    fn a_bookmark_that_cannot_be_placed_is_kept_at_the_start_and_flagged() {
        let (mut books, mut l, loc) = setup();
        let r = merge_incoming(
            &mut books,
            &[mark_change("b", 7, 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(r.bookmarks, 1);
        let b = &books[0].bookmarks[0];
        assert_eq!((b.spine, b.frac), (0, 0.0));
        assert!(
            l.bookmarks[&key(DOC, "b")].approx,
            "flagged as a rough place"
        );
    }

    #[test]
    fn a_bookmark_deleted_in_bookx_is_not_brought_back_and_one_deleted_on_the_device_is_kept() {
        let (mut books, mut l, _) = setup();
        let loc = ByPage;
        merge_incoming(
            &mut books,
            &[mark_change("b", 7, 100, 1), mark_change("c", 30, 100, 2)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(books[0].bookmarks.len(), 2);
        // the reader deletes the first one; the device sends it again, changed
        books[0].bookmarks.retain(|b| b.label != "Page 7");
        let r = merge_incoming(
            &mut books,
            &[mark_change("b", 7, 500, 3)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(r.bookmarks, 0);
        assert_eq!(books[0].bookmarks.len(), 1, "the tombstone holds");
        // the device deletes the other one: it stays here
        let mut gone = mark_change("c", 30, 600, 4);
        gone.record.annotation.deleted = true;
        let r = merge_incoming(&mut books, &[gone], &mut l, &loc, now());
        assert_eq!(r.removed, 0);
        assert_eq!(books[0].bookmarks.len(), 1);
        assert_eq!(books[0].bookmarks[0].label, "Page 30");
    }

    #[test]
    fn a_bookmark_an_older_sync_only_remembered_is_imported_now() {
        let (mut books, mut l, _) = setup();
        l.incoming.insert(
            key(DOC, "b"),
            Incoming {
                book_id: 1,
                highlight: None,
                source: "KOReader (Kobo Libra)".into(),
                note: String::new(),
                device_look: Look::default(),
                updated: 100,
                seq: 1,
                state: State::Bookmark,
                text: String::new(),
                made_here: false,
                hint: Hint::default(),
            },
        );
        let r = merge_incoming(
            &mut books,
            &[mark_change("b", 7, 100, 1)],
            &mut l,
            &exact(2, 0.4),
            now(),
        );
        assert_eq!(r.bookmarks, 1);
        // and only once
        let r = merge_incoming(
            &mut books,
            &[mark_change("b", 7, 100, 1)],
            &mut l,
            &exact(2, 0.4),
            now(),
        );
        assert_eq!(r.bookmarks, 0);
        assert_eq!(books[0].bookmarks.len(), 1);
    }

    #[test]
    fn bookmarks_do_not_disturb_highlights_and_their_colours() {
        let (mut books, mut l, loc) = setup();
        let mut hl = change("a", "Fear is the mind-killer.", "n", 100, 1);
        hl.record.annotation.color = "purple".into();
        let r = merge_incoming(
            &mut books,
            &[hl, mark_change("b", 7, 100, 2)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!((r.added, r.bookmarks), (1, 1));
        assert_eq!(books[0].highlights[0].color, "purple");
        assert_eq!(books[0].highlights.len(), 1);
    }

    #[test]
    fn unknown_documents_are_counted() {
        let (mut books, mut l, loc) = setup();
        l.docs.clear();
        let r = merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(r.unmatched, 1);
        assert!(l.incoming.is_empty());
    }

    #[test]
    fn changes_from_bookx_itself_are_not_merged_back() {
        let (mut books, mut l, loc) = setup();
        let mut c = change("a", "Fear is the mind-killer.", "", 100, 1);
        c.record.origin = ORIGIN_BOOKX.into();
        merge_incoming(&mut books, &[c], &mut l, &loc, now());
        assert!(books[0].highlights.is_empty());
    }

    #[test]
    fn outgoing_sends_only_new_and_changed_and_tombstones() {
        let (mut books, mut l, loc) = setup();
        books[0]
            .highlights
            .push(books_hl("hl-1", "Something I highlighted here."));
        let o = outgoing(&books, &l, 1000);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].1.len(), 1);
        assert_eq!(o[0].1[0].color, "yellow");
        assert_eq!(o[0].1[0].drawer, "lighten");
        commit_outgoing(&mut l, o.into_iter().next().unwrap().2);
        assert!(outgoing(&books, &l, 1001).is_empty());
        books[0].highlights[0].note = "now with a note".into();
        let o = outgoing(&books, &l, 2000);
        assert_eq!(o[0].1[0].updated, 2000);
        commit_outgoing(&mut l, o.into_iter().next().unwrap().2);
        books[0].highlights.clear();
        let o = outgoing(&books, &l, 3000);
        assert!(o[0].1[0].deleted);
        commit_outgoing(&mut l, o.into_iter().next().unwrap().2);
        assert!(outgoing(&books, &l, 3001).is_empty());
        let _ = loc;
    }

    #[test]
    fn highlights_from_the_device_are_not_sent_back_and_other_books_are_not_published() {
        let (mut books, mut l, loc) = setup();
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        assert!(outgoing(&books, &l, 1000).is_empty());
        let mut other = book();
        other.id = 2;
        other
            .highlights
            .push(books_hl("o1", "private quote of another book"));
        books.push(other);
        assert!(
            outgoing(&books, &l, 1000).is_empty(),
            "book 2 has no device document"
        );
    }

    #[test]
    fn documents_match_by_digest_then_by_title() {
        let books = vec![book()];
        let mut l = Ledger::default();
        let docs = vec![(
            "u".to_string(),
            DOC.to_string(),
            "Other".to_string(),
            String::new(),
        )];
        let un = resolve_docs(Path::new("/lib"), &books, &docs, &mut l, &|_| {
            Some(DOC.to_string())
        });
        assert!(un.is_empty());
        assert_eq!(l.docs.get(DOC), Some(&1));
        // by title when the digest differs (a converted copy on the device)
        let mut l = Ledger::default();
        let docs = vec![(
            "u".to_string(),
            DOC.to_string(),
            "Dune".to_string(),
            "Frank Herbert".to_string(),
        )];
        let un = resolve_docs(Path::new("/lib"), &books, &docs, &mut l, &|_| {
            Some("f".repeat(32))
        });
        assert!(un.is_empty());
        assert_eq!(l.docs.get(DOC), Some(&1));
        // another author is not the same book
        let mut l = Ledger::default();
        let docs = vec![(
            "u".to_string(),
            DOC.to_string(),
            "Dune".to_string(),
            "Someone Else".to_string(),
        )];
        let un = resolve_docs(Path::new("/lib"), &books, &docs, &mut l, &|_| None);
        assert_eq!(un.len(), 1);
    }

    fn colored(id: &str, text: &str, color: &str, drawer: &str, updated: i64, seq: u64) -> Change {
        let mut c = change(id, text, "", updated, seq);
        c.record.annotation.color = color.into();
        c.record.annotation.drawer = drawer.into();
        c
    }

    /// The owner's report: the device had purple highlights but Bookx kept them yellow, even
    /// though the annotation was already linked. Recolouring on the device must reach the library.
    #[test]
    fn a_recolour_on_the_device_changes_a_highlight_that_was_already_linked() {
        let (mut books, mut l, loc) = setup();
        merge_incoming(
            &mut books,
            &[colored(
                "a",
                "Fear is the mind-killer.",
                "yellow",
                "lighten",
                100,
                1,
            )],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(books[0].highlights[0].color, "yellow");
        let r = merge_incoming(
            &mut books,
            &[colored(
                "a",
                "Fear is the mind-killer.",
                "purple",
                "lighten",
                200,
                2,
            )],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(r.looks, 1, "{r:?}");
        assert_eq!(books[0].highlights[0].color, "purple");
        // an unchanged device record never resets a look the reader chose in Bookx
        books[0].highlights[0].color = "red".into();
        let r = merge_incoming(
            &mut books,
            &[colored(
                "a",
                "Fear is the mind-killer.",
                "purple",
                "lighten",
                300,
                3,
            )],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(r.looks, 0, "the device did not change its colour: {r:?}");
        assert_eq!(books[0].highlights[0].color, "red");
        // but a later change on the device wins
        let r = merge_incoming(
            &mut books,
            &[colored(
                "a",
                "Fear is the mind-killer.",
                "cyan",
                "lighten",
                400,
                4,
            )],
            &mut l,
            &loc,
            now(),
        );
        assert_eq!(r.looks, 1);
        assert_eq!(books[0].highlights[0].color, "cyan");
    }

    /// A forced Sync now must repair old imports even when the server change number did not move.
    /// Colours are taken from the real sidecar of the owner's *Building Microservices*: 406
    /// highlights in eight colours. Here six of them stand for highlights linked earlier with
    /// Bookx's default yellow.
    #[test]
    fn a_forced_merge_repairs_stale_default_yellow_without_a_new_server_change() {
        let (mut books, mut l, loc) = setup();
        let texts = [
            "Fear is the mind-killer.",
            "A second long quotation.",
            "Fear is the mind-killer, again.",
            "A fourth quotation.",
            "A fifth quotation.",
            "A sixth quotation.",
        ];
        for (i, text) in texts.iter().enumerate() {
            books[0]
                .highlights
                .push(mine(&format!("h{i}"), text, 10 + i as i32 * 60));
        }
        let colors = ["purple", "cyan", "olive", "red", "orange", "green"];
        let changes: Vec<Change> = colors
            .iter()
            .zip(texts.iter())
            .enumerate()
            .map(|(i, (color, text))| colored(&format!("a{i}"), text, color, "lighten", 100, 1))
            .collect();
        // the old merge linked them when the server had no colour: same seq, default yellow
        let yellow: Vec<Change> = changes
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.record.annotation.color = "yellow".into();
                c.record.annotation.drawer = "lighten".into();
                c
            })
            .collect();
        merge_incoming(&mut books, &yellow, &mut l, &loc, now());
        assert!(
            books[0].highlights.iter().all(|h| h.color == "yellow"),
            "what the owner saw"
        );
        // a repeat is skipped: the server change number has not moved
        assert_eq!(
            merge_incoming(&mut books, &changes, &mut l, &loc, now()),
            Report::default()
        );
        assert_eq!(
            crate::annotation_sync::repair_looks(&mut books, &changes, &mut l),
            6
        );
        let got: Vec<_> = books[0]
            .highlights
            .iter()
            .map(|h| h.color.as_str())
            .collect();
        assert_eq!(
            got, colors,
            "every colour survives, including olive and cyan"
        );
        // a second pass changes nothing
        assert_eq!(
            crate::annotation_sync::repair_looks(&mut books, &changes, &mut l),
            0
        );
        assert_eq!(
            merge_incoming(&mut books, &changes, &mut l, &loc, now()),
            Report::default()
        );
    }

    /// Root cause R1 (docs/highlight-sync-rca.md): the wireless path had its own colour and style
    /// table. A word it did not know became yellow and plain; here it must arrive as written, and a
    /// repeat must not count it as a change every time.
    #[test]
    fn a_colour_and_drawer_nobody_knows_arrive_as_written_and_a_repeat_changes_nothing() {
        let (mut books, mut l, loc) = setup();
        let first = [colored(
            "a",
            "Fear is the mind-killer.",
            "teal",
            "wavy",
            100,
            1,
        )];
        merge_incoming(&mut books, &first, &mut l, &loc, now());
        let h = &books[0].highlights[0];
        assert_eq!((h.color.as_str(), h.style.as_str()), ("teal", "wavy"));
        assert_eq!(repair_looks(&mut books, &first, &mut l), 0);
        let again = [colored(
            "a",
            "Fear is the mind-killer.",
            "teal",
            "wavy",
            200,
            2,
        )];
        let r = merge_incoming(&mut books, &again, &mut l, &loc, now());
        assert_eq!(r.looks, 0, "{r:?}");
    }

    /// Root cause R2: a highlight made in Bookx went to the server as yellow whenever its colour was
    /// not one of the eight names (a custom colour, an extra word).
    #[test]
    fn what_bookx_sends_carries_the_nearest_koreader_colour_and_drawer() {
        let (mut books, l, _loc) = setup();
        for (id, color, style) in [
            ("c1", "#ff0000", "squiggly"),
            ("c2", "pink", ""),
            ("c3", "Grey", "strikethrough"),
            ("c4", "cyan", "invert"),
            ("c5", "teal", "wavy"),
        ] {
            let mut h = books_hl(id, &format!("A highlight made in Bookx: {id} {color}"));
            h.color = color.into();
            h.style = style.into();
            books[0].highlights.push(h);
        }
        let o = outgoing(&books, &l, 1000);
        for a in &o[0].1 {
            let want = match a.id.as_str() {
                "c1" => ("red", "underscore"),
                "c2" => ("red", "lighten"),
                "c3" => ("gray", "strikeout"),
                "c4" => ("cyan", "invert"),
                _ => ("teal", "wavy"),
            };
            assert_eq!((a.color.as_str(), a.drawer.as_str()), want, "{}", a.id);
        }
    }

    /// Root cause R3: the change fingerprint compared the raw words, so `""` becoming `background`
    /// (the same look) was sent to the device as an update.
    #[test]
    fn a_spelling_of_the_same_look_is_not_a_change_to_send() {
        let (mut books, mut l, _loc) = setup();
        books[0]
            .highlights
            .push(books_hl("hl-1", "Something I highlighted here."));
        let o = outgoing(&books, &l, 1000);
        commit_outgoing(&mut l, o.into_iter().next().unwrap().2);
        books[0].highlights[0].style = "background".into();
        books[0].highlights[0].color = "Yellow".into();
        assert!(outgoing(&books, &l, 2000).is_empty());
        books[0].highlights[0].color = "purple".into();
        assert_eq!(outgoing(&books, &l, 3000)[0].1[0].color, "purple");
    }

    #[test]
    fn colours_and_styles_map_both_ways() {
        // KOReader's olive and cyan are kept as written (they were turned into green and blue)
        assert_eq!(color_in("olive"), "olive");
        assert_eq!(color_in("cyan"), "cyan");
        assert_eq!(color_out("cyan"), "cyan");
        assert_eq!(color_in("weird"), "weird", "a word nobody knows is kept");
        assert_eq!(color_out("weird"), "weird");
        assert_eq!(color_in(""), "yellow");
        assert_eq!(color_out("pink"), "red");
        assert_eq!(style_in("lighten"), "background");
        assert_eq!(style_in("whatever"), "whatever");
        assert_eq!(drawer_out("strikethrough"), "strikeout");
        assert_eq!(
            color_out("#ff0000"),
            "red",
            "a custom colour is not sent as yellow"
        );
    }

    #[test]
    fn ledger_round_trips() {
        let dir = std::env::temp_dir().join(format!("bookx-annledger-{}", uuid::Uuid::new_v4()));
        let p = ledger_path(&dir);
        let (mut books, mut l, loc) = setup();
        merge_incoming(
            &mut books,
            &[change("a", "Fear is the mind-killer.", "", 100, 1)],
            &mut l,
            &loc,
            now(),
        );
        l.save(&p).unwrap();
        assert_eq!(Ledger::load(&p), l);
        assert_eq!(Ledger::load(&dir.join("missing.json")), Ledger::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn report_text() {
        let r = Report {
            added: 2,
            unplaced: 1,
            ..Report::default()
        };
        assert!(r.text().contains("2 highlights"));
        assert!(r.text().contains("not found"));
        assert_eq!(Report::default().text(), "Nothing new");
    }
}
