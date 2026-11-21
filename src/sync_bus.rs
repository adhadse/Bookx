//! Live updates after a sync. Highlights, notes, bookmarks, colours and reading positions arrive
//! from a Kobo or other e-reader (over Wi-Fi through the built-in server, or over the cable) while a
//! book is open; this module is how the open reader and the library window hear of it.
//!
//! * **From the server threads**: the handlers call [`server_listener`]'s closure on a worker thread.
//!   It only queues the [`Note`] in a [`Mailbox`] and wakes the main loop; no GTK call happens there.
//! * **On the main thread**: `annotation_sync_ui::live` plans the work from the queued notes
//!   ([`plan`]), merges into the library and publishes one typed [`SyncEvent`] per book and device
//!   with [`publish`]. Events are coalesced ([`Coalescer`]: a burst inside 250 ms is one event) and
//!   handed to the subscribers of the book ([`subscribe`]).
//! * What changed is found by comparing [`Snapshot`]s of the books before and after, so the cable
//!   sync, the plugin sync and an import all produce the same events whatever they did.
//!
//! The data structures are plain Rust and unit tested; the glib glue at the bottom is small.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use bookx_serve::notify::{Listener, Note};

use crate::core::library::Book;

/// A burst of events inside this time is one event.
pub const DEBOUNCE_MS: u64 = 250;
/// A burst that never pauses is still delivered after this long.
pub const MAX_WAIT_MS: u64 = 1000;

// -- what changed ---------------------------------------------------------------------------------

/// The kinds of change, as bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Kinds(u8);

impl Kinds {
    pub const NONE: Kinds = Kinds(0);
    /// Highlights added, removed or moved.
    pub const HIGHLIGHTS: Kinds = Kinds(1);
    /// The note of a highlight.
    pub const NOTES: Kinds = Kinds(2);
    pub const BOOKMARKS: Kinds = Kinds(4);
    /// The reading position of the library, or a device that is ahead.
    pub const PROGRESS: Kinds = Kinds(8);
    /// Colour or style of a highlight.
    pub const LOOK: Kinds = Kinds(16);

    pub fn contains(self, other: Kinds) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }

    pub fn intersects(self, other: Kinds) -> bool {
        self.0 & other.0 != 0
    }

    pub fn union(self, other: Kinds) -> Kinds {
        Kinds(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the annotations on a page or in the sidebar lists may have changed.
    pub fn annotations(self) -> bool {
        self.intersects(
            Kinds::HIGHLIGHTS
                .union(Kinds::NOTES)
                .union(Kinds::BOOKMARKS)
                .union(Kinds::LOOK),
        )
    }

    /// `highlights,notes` for the dev commands and logs.
    pub fn names(self) -> String {
        let all = [
            (Kinds::HIGHLIGHTS, "highlights"),
            (Kinds::NOTES, "notes"),
            (Kinds::BOOKMARKS, "bookmarks"),
            (Kinds::PROGRESS, "progress"),
            (Kinds::LOOK, "look"),
        ];
        let v: Vec<&str> = all
            .iter()
            .filter(|(k, _)| self.contains(*k))
            .map(|(_, n)| *n)
            .collect();
        if v.is_empty() {
            "none".into()
        } else {
            v.join(",")
        }
    }
}

impl std::ops::BitOr for Kinds {
    type Output = Kinds;
    fn bitor(self, o: Kinds) -> Kinds {
        self.union(o)
    }
}

/// How the change reached Bookx.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Via {
    /// A device talked to the built-in server.
    #[default]
    Wifi,
    /// A device synced by the cable.
    Cable,
    /// An import (Calibre, Readwise, a file of notes).
    Import,
}

/// A device that is further along in a book than the library.
#[derive(Debug, Clone, PartialEq)]
pub struct Ahead {
    pub percentage: f64,
    pub device_id: String,
    /// KOReader's own place (an xpointer), kept for the history; the percentage decides.
    pub progress: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncEvent {
    pub book_id: u64,
    pub kinds: Kinds,
    /// The device's name, empty when it is not known.
    pub device: String,
    pub via: Via,
    /// A position a device reported (it is *not* applied to the library).
    pub ahead: Option<Ahead>,
}

impl SyncEvent {
    pub fn new(book_id: u64, kinds: Kinds, device: &str, via: Via) -> SyncEvent {
        SyncEvent {
            book_id,
            kinds,
            device: device.to_string(),
            via,
            ahead: None,
        }
    }
}

// -- coalescing -----------------------------------------------------------------------------------

/// Gathers events and hands them out once things have been quiet for [`DEBOUNCE_MS`] (or
/// [`MAX_WAIT_MS`] have passed since the first). Events of the same book and device are one;
/// the order is that of the first arrival. Times are milliseconds from any steady clock.
#[derive(Debug, Default)]
pub struct Coalescer {
    pending: Vec<SyncEvent>,
    first: Option<u64>,
    last: Option<u64>,
}

impl Coalescer {
    pub fn push(&mut self, ev: SyncEvent, now: u64) {
        if self.pending.is_empty() {
            self.first = Some(now);
        }
        self.last = Some(now);
        match self
            .pending
            .iter_mut()
            .find(|p| p.book_id == ev.book_id && p.device == ev.device)
        {
            Some(p) => {
                p.kinds = p.kinds | ev.kinds;
                p.via = ev.via;
                if ev.ahead.is_some() {
                    p.ahead = ev.ahead;
                }
            }
            None => self.pending.push(ev),
        }
    }

    /// Whether [`Coalescer::take`] should be called now.
    pub fn due(&self, now: u64) -> bool {
        match (self.first, self.last) {
            (Some(f), Some(l)) if !self.pending.is_empty() => {
                now >= l + DEBOUNCE_MS || now >= f + MAX_WAIT_MS
            }
            _ => false,
        }
    }

    /// Everything gathered, in order; the coalescer is empty afterwards.
    pub fn take(&mut self) -> Vec<SyncEvent> {
        self.first = None;
        self.last = None;
        std::mem::take(&mut self.pending)
    }
}

// -- snapshots: what a merge changed ----------------------------------------------------------------

/// What one book holds, as one hash per kind of change.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BookSig {
    highlights: u64,
    notes: u64,
    look: u64,
    bookmarks: u64,
    progress: u64,
}

fn h<T: Hash>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

/// The hashes of one book. Order does not matter (the items' hashes are added).
pub fn sig_of(b: &Book) -> BookSig {
    let mut s = BookSig::default();
    for x in &b.highlights {
        s.highlights = s
            .highlights
            .wrapping_add(h(&(&x.id, x.spine, x.start, x.end)));
        s.notes = s.notes.wrapping_add(h(&(&x.id, &x.note)));
        s.look = s.look.wrapping_add(h(&(&x.id, &x.color, &x.style)));
    }
    for m in &b.bookmarks {
        s.bookmarks = s
            .bookmarks
            .wrapping_add(h(&(&m.id, m.spine, m.frac.to_bits(), &m.label)));
    }
    s.progress = b.progress.to_bits();
    s
}

/// The kinds that differ between two states of a book.
pub fn kinds_between(a: &BookSig, b: &BookSig) -> Kinds {
    let mut k = Kinds::NONE;
    if a.highlights != b.highlights {
        k = k | Kinds::HIGHLIGHTS;
    }
    if a.notes != b.notes {
        k = k | Kinds::NOTES;
    }
    if a.look != b.look {
        k = k | Kinds::LOOK;
    }
    if a.bookmarks != b.bookmarks {
        k = k | Kinds::BOOKMARKS;
    }
    if a.progress != b.progress {
        k = k | Kinds::PROGRESS;
    }
    k
}

/// The state of every book at one moment.
#[derive(Debug, Default)]
pub struct Snapshot(HashMap<u64, BookSig>);

impl Snapshot {
    pub fn take(books: &[Book]) -> Snapshot {
        Snapshot(books.iter().map(|b| (b.id, sig_of(b))).collect())
    }

    /// One event per book that is different now (a new book counts as everything it holds),
    /// ordered by book id.
    pub fn events(&self, books: &[Book], device: &str, via: Via) -> Vec<SyncEvent> {
        let mut out: Vec<SyncEvent> = books
            .iter()
            .filter_map(|b| {
                let now = sig_of(b);
                let before = self.0.get(&b.id).copied().unwrap_or_default();
                let k = kinds_between(&before, &now);
                (!k.is_empty()).then(|| SyncEvent::new(b.id, k, device, via))
            })
            .collect();
        out.sort_by_key(|e| e.book_id);
        out
    }
}

// -- from the server threads to the main loop --------------------------------------------------------

/// A queue that any thread may push to and the main thread empties. When more than `cap` notes
/// wait, the oldest are dropped (every note only means "look at the store again").
pub struct Mailbox<T> {
    items: Mutex<Vec<T>>,
    cap: usize,
}

impl<T> Mailbox<T> {
    pub const fn new(cap: usize) -> Mailbox<T> {
        Mailbox {
            items: Mutex::new(Vec::new()),
            cap,
        }
    }

    pub fn push(&self, t: T) {
        let mut v = self.items.lock().unwrap_or_else(|p| p.into_inner());
        if v.len() >= self.cap {
            v.remove(0);
        }
        v.push(t);
    }

    /// Everything queued, oldest first.
    pub fn drain(&self) -> Vec<T> {
        std::mem::take(&mut *self.items.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// What the main thread does with a batch of notes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Merge what devices sent into the library.
    pub merge: bool,
    /// The newest reported position of each document and device, oldest first.
    pub positions: Vec<PositionNote>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PositionNote {
    pub document: String,
    pub device: String,
    pub device_id: String,
    pub percentage: f64,
    pub progress: String,
}

/// Reduces a batch of notes: one merge for all annotation notes, the last position of each
/// document and device.
pub fn plan(notes: &[Note]) -> Plan {
    let mut p = Plan::default();
    for n in notes {
        match n {
            Note::Annotations { .. } => p.merge = true,
            Note::Progress {
                document,
                device,
                device_id,
                percentage,
                progress,
                ..
            } => {
                let new = PositionNote {
                    document: document.clone(),
                    device: device.clone(),
                    device_id: device_id.clone(),
                    percentage: *percentage,
                    progress: progress.clone(),
                };
                match p
                    .positions
                    .iter_mut()
                    .find(|x| x.document == *document && x.device == *device)
                {
                    Some(x) => *x = new,
                    None => p.positions.push(new),
                }
            }
        }
    }
    p
}

static MAILBOX: Mailbox<Note> = Mailbox::new(1024);
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static SCHEDULED: AtomicBool = AtomicBool::new(false);

/// The listener to give to the server handlers (`on_change`): queues the note and wakes the main
/// loop. Safe to call on any thread, never touches GTK.
pub fn server_listener() -> Listener {
    Arc::new(|note: &Note| {
        MAILBOX.push(note.clone());
        if !SCHEDULED.swap(true, Ordering::SeqCst) {
            if let Some(w) = WAKE.get() {
                w();
            }
        }
    })
}

thread_local! {
    /// What was delivered, for the tests (`koannot:events`).
    static DEV_LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static HANDLER: RefCell<Option<Rc<dyn Fn(Vec<Note>)>>> = const { RefCell::new(None) };
    static HUB: Hub = Hub::default();
}

fn deliver_mail() {
    SCHEDULED.store(false, Ordering::SeqCst);
    let notes = MAILBOX.drain();
    if notes.is_empty() {
        return;
    }
    if let Some(f) = HANDLER.with(|h| h.borrow().clone()) {
        f(notes);
    }
}

/// Call once on the main thread: `handler` gets every batch of server notes, on the main thread.
pub fn install(handler: impl Fn(Vec<Note>) + 'static) {
    HANDLER.with(|h| *h.borrow_mut() = Some(Rc::new(handler)));
    let ctx = relm4::gtk::glib::MainContext::default();
    let _ = WAKE.set(Box::new(move || {
        ctx.invoke(deliver_mail);
    }));
    // notes that arrived before the handler existed
    if !MAILBOX.drain_is_empty() {
        relm4::gtk::glib::idle_add_local_once(deliver_mail);
    }
}

impl Mailbox<Note> {
    fn drain_is_empty(&self) -> bool {
        self.items
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    }
}

// -- the subscribers (main thread) -----------------------------------------------------------------

type Callback = Rc<dyn Fn(&SyncEvent)>;

#[derive(Default)]
struct Hub {
    subs: RefCell<Vec<(u64, Option<u64>, Callback)>>,
    next: Cell<u64>,
    co: RefCell<Coalescer>,
    timer: RefCell<Option<relm4::gtk::glib::SourceId>>,
}

/// Keeps a subscription while it lives; dropping it (a reader closed) ends it.
pub struct Subscription {
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = HUB.try_with(|hub| hub.subs.borrow_mut().retain(|(i, _, _)| *i != self.id));
    }
}

/// Calls `f` for the events of `book` (every book when `None`), on the main thread.
pub fn subscribe(book: Option<u64>, f: impl Fn(&SyncEvent) + 'static) -> Subscription {
    HUB.with(|hub| {
        let id = hub.next.get();
        hub.next.set(id + 1);
        hub.subs.borrow_mut().push((id, book, Rc::new(f)));
        Subscription { id }
    })
}

fn now_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Test hook: the events delivered since the last call, one line each.
pub fn dev_log_take() -> Vec<String> {
    DEV_LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// Whether a reader of this book is open (it listens for the events of the book).
pub fn has_reader_for(book: u64) -> bool {
    HUB.with(|hub| hub.subs.borrow().iter().any(|(_, b, _)| *b == Some(book)))
}

/// Hands the events of the gathered window to the subscribers right away (a subscriber may
/// subscribe or unsubscribe meanwhile). Returns how many events were delivered.
pub fn flush() -> usize {
    let events = HUB.with(|hub| hub.co.borrow_mut().take());
    let n = events.len();
    DEV_LOG.with(|l| {
        let mut l = l.borrow_mut();
        for e in &events {
            if l.len() >= 200 {
                l.remove(0);
            }
            l.push(format!(
                "book={} kinds={} device={:?} via={:?} ahead={}",
                e.book_id,
                e.kinds.names(),
                e.device,
                e.via,
                e.ahead
                    .as_ref()
                    .map_or("none".to_string(), |a| format!("{:.2}", a.percentage))
            ));
        }
    });
    for ev in events {
        let subs: Vec<(Option<u64>, Callback)> = HUB.with(|hub| {
            hub.subs
                .borrow()
                .iter()
                .map(|(_, b, f)| (*b, f.clone()))
                .collect()
        });
        for (book, f) in subs {
            if book.is_none() || book == Some(ev.book_id) {
                f(&ev);
            }
        }
    }
    n
}

/// Queues an event; subscribers hear it after the debounce window (events of one burst arrive as
/// one). Must be called on the main thread.
pub fn publish(ev: SyncEvent) {
    HUB.with(|hub| {
        hub.co.borrow_mut().push(ev, now_ms());
        // without a main loop on this thread (a unit test) the event waits for `flush`
        if !relm4::gtk::glib::MainContext::default().is_owner() {
            return;
        }
        if hub.timer.borrow().is_none() {
            let id =
                relm4::gtk::glib::timeout_add_local(std::time::Duration::from_millis(50), || {
                    let due = HUB.with(|hub| hub.co.borrow().due(now_ms()));
                    if due {
                        HUB.with(|hub| hub.timer.borrow_mut().take());
                        flush();
                        return relm4::gtk::glib::ControlFlow::Break;
                    }
                    relm4::gtk::glib::ControlFlow::Continue
                });
            *hub.timer.borrow_mut() = Some(id);
        }
    });
}

/// Queues the events of a snapshot comparison.
pub fn publish_changes(before: &Snapshot, books: &[Book], device: &str, via: Via) -> usize {
    let evs = before.events(books, device, via);
    let n = evs.len();
    for e in evs {
        publish(e);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::{Bookmark, Highlight};
    use chrono::Utc;

    fn ev(book: u64, k: Kinds, dev: &str) -> SyncEvent {
        SyncEvent::new(book, k, dev, Via::Wifi)
    }

    fn hl(id: &str, color: &str, note: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start: 1,
            end: 5,
            color: color.into(),
            style: String::new(),
            note: note.into(),
            text: "text".into(),
            created: Utc::now(),
        }
    }

    fn book(id: u64) -> Book {
        let now = Utc::now();
        Book {
            id,
            file: format!("{id}.epub"),
            cover: None,
            meta: Default::default(),
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
        }
    }

    #[test]
    fn a_burst_is_one_event_per_book_and_device_in_order_of_arrival() {
        let mut c = Coalescer::default();
        c.push(ev(2, Kinds::HIGHLIGHTS, "Kobo"), 0);
        c.push(ev(1, Kinds::NOTES, "Kobo"), 10);
        c.push(ev(2, Kinds::LOOK, "Kobo"), 20);
        c.push(ev(2, Kinds::BOOKMARKS, "Other"), 30);
        assert!(!c.due(100), "still inside the quiet window");
        assert!(c.due(30 + DEBOUNCE_MS));
        let out = c.take();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].book_id, 2);
        assert_eq!(out[0].kinds, Kinds::HIGHLIGHTS | Kinds::LOOK);
        assert_eq!(out[1].book_id, 1);
        assert_eq!(out[2].device, "Other");
        assert!(!c.due(10_000));
    }

    #[test]
    fn a_burst_that_never_pauses_is_delivered_after_the_longest_wait() {
        let mut c = Coalescer::default();
        for i in 0..40 {
            c.push(ev(1, Kinds::NOTES, "K"), i * 100);
            if i * 100 < MAX_WAIT_MS {
                assert!(!c.due(i * 100), "{i}");
            }
        }
        assert!(c.due(MAX_WAIT_MS));
    }

    #[test]
    fn the_newest_position_of_a_device_wins_in_a_coalesced_event() {
        let mut c = Coalescer::default();
        let mut a = ev(1, Kinds::PROGRESS, "K");
        a.ahead = Some(Ahead {
            percentage: 0.4,
            device_id: "d".into(),
            progress: String::new(),
        });
        let mut b = a.clone();
        b.ahead.as_mut().unwrap().percentage = 0.62;
        c.push(a, 0);
        c.push(b, 5);
        c.push(ev(1, Kinds::NOTES, "K"), 6);
        let out = c.take();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ahead.as_ref().unwrap().percentage, 0.62);
        assert!(out[0].kinds.contains(Kinds::PROGRESS) && out[0].kinds.contains(Kinds::NOTES));
    }

    #[test]
    fn kinds_name_and_test_what_they_hold() {
        let k = Kinds::NOTES | Kinds::LOOK;
        assert!(k.contains(Kinds::NOTES) && k.annotations());
        assert!(!k.contains(Kinds::PROGRESS));
        assert!(!Kinds::PROGRESS.annotations());
        assert_eq!(k.names(), "notes,look");
        assert_eq!(Kinds::NONE.names(), "none");
        assert!(!Kinds::NONE.contains(Kinds::NONE));
    }

    #[test]
    fn a_snapshot_finds_exactly_what_a_merge_changed() {
        let mut books = vec![book(1), book(2)];
        books[0].highlights = vec![hl("a", "yellow", ""), hl("b", "green", "")];
        let before = Snapshot::take(&books);
        assert!(before.events(&books, "K", Via::Wifi).is_empty());

        books[0].highlights[0].color = "purple".into();
        let e = before.events(&books, "K", Via::Wifi);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].kinds, Kinds::LOOK);

        books[0].highlights[1].note = "hello".into();
        books[0].highlights.push(hl("c", "blue", ""));
        books[1].bookmarks.push(Bookmark {
            id: "x".into(),
            spine: 1,
            frac: 0.5,
            label: "p".into(),
            created: Utc::now(),
        });
        books[1].progress = 0.3;
        let e = before.events(&books, "Libra", Via::Cable);
        assert_eq!(e.len(), 2);
        assert_eq!(
            e[0].kinds,
            Kinds::HIGHLIGHTS | Kinds::NOTES | Kinds::LOOK,
            "{}",
            e[0].kinds.names()
        );
        assert_eq!(e[1].kinds, Kinds::BOOKMARKS | Kinds::PROGRESS);
        assert_eq!((e[1].device.as_str(), e[1].via), ("Libra", Via::Cable));

        books[0].highlights.retain(|h| h.id != "c");
        books[0].highlights.reverse();
        let again = Snapshot::take(&books);
        books[0].highlights.reverse();
        assert!(
            again.events(&books, "", Via::Wifi).is_empty(),
            "order is not a change"
        );
    }

    #[test]
    fn the_mailbox_keeps_order_per_thread_and_loses_nothing_below_the_cap() {
        let mb = Arc::new(Mailbox::<(u32, u32)>::new(10_000));
        std::thread::scope(|s| {
            for t in 0..8u32 {
                let mb = mb.clone();
                s.spawn(move || {
                    for i in 0..200 {
                        mb.push((t, i));
                    }
                });
            }
        });
        let all = mb.drain();
        assert_eq!(all.len(), 1600);
        for t in 0..8 {
            let mine: Vec<u32> = all
                .iter()
                .filter(|(x, _)| *x == t)
                .map(|(_, i)| *i)
                .collect();
            assert_eq!(mine, (0..200).collect::<Vec<_>>());
        }
        assert!(mb.drain().is_empty());
    }

    #[test]
    fn a_full_mailbox_drops_the_oldest_and_a_poisoned_one_still_works() {
        let mb = Arc::new(Mailbox::<u32>::new(3));
        for i in 0..5 {
            mb.push(i);
        }
        assert_eq!(mb.drain(), vec![2, 3, 4]);
        let m2 = mb.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.items.lock().unwrap();
            panic!("poison");
        })
        .join();
        mb.push(9);
        assert_eq!(mb.drain(), vec![9]);
    }

    fn ann(doc: &str) -> Note {
        Note::Annotations {
            user: "u".into(),
            document: doc.into(),
            device: "K".into(),
            seq: 1,
            highlights: 1,
            bookmarks: 0,
            notes: 0,
            removed: 0,
        }
    }

    fn pos(doc: &str, dev: &str, pct: f64) -> Note {
        Note::Progress {
            user: "u".into(),
            document: doc.into(),
            device: dev.into(),
            device_id: "id".into(),
            percentage: pct,
            progress: String::new(),
        }
    }

    #[test]
    fn a_batch_of_notes_is_one_merge_and_the_last_position_per_device() {
        let p = plan(&[
            ann("a"),
            pos("d1", "Kobo", 0.2),
            ann("b"),
            pos("d1", "Kobo", 0.6),
            pos("d1", "Boox", 0.3),
        ]);
        assert!(p.merge);
        assert_eq!(p.positions.len(), 2);
        assert_eq!(p.positions[0].percentage, 0.6);
        assert_eq!(p.positions[1].device, "Boox");
        assert!(!plan(&[pos("d", "K", 0.1)]).merge);
        assert_eq!(plan(&[]), Plan::default());
    }

    #[test]
    fn subscribers_hear_their_book_only_and_a_dropped_subscription_hears_nothing() {
        use std::cell::RefCell;
        let heard: Rc<RefCell<Vec<u64>>> = Rc::default();
        let (h1, h2) = (heard.clone(), heard.clone());
        let s1 = subscribe(Some(1), move |e| h1.borrow_mut().push(e.book_id));
        let _s2 = subscribe(None, move |e| h2.borrow_mut().push(100 + e.book_id));
        HUB.with(|hub| {
            let mut c = hub.co.borrow_mut();
            c.push(ev(1, Kinds::NOTES, "K"), 0);
            c.push(ev(2, Kinds::NOTES, "K"), 0);
        });
        assert_eq!(flush(), 2);
        assert_eq!(*heard.borrow(), vec![1, 101, 102]);
        drop(s1);
        heard.borrow_mut().clear();
        HUB.with(|hub| hub.co.borrow_mut().push(ev(1, Kinds::NOTES, "K"), 0));
        flush();
        assert_eq!(*heard.borrow(), vec![101]);
    }

    #[test]
    fn a_subscriber_may_end_its_own_subscription_while_it_is_called() {
        let slot: Rc<RefCell<Option<Subscription>>> = Rc::default();
        let (s2, n) = (slot.clone(), Rc::new(Cell::new(0)));
        let n2 = n.clone();
        *slot.borrow_mut() = Some(subscribe(None, move |_| {
            n2.set(n2.get() + 1);
            s2.borrow_mut().take();
        }));
        for _ in 0..2 {
            HUB.with(|hub| hub.co.borrow_mut().push(ev(1, Kinds::NOTES, "K"), 0));
            flush();
        }
        assert_eq!(n.get(), 1);
    }
}
