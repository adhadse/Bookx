//! Restoring highlights, notes and bookmarks from the annotation history, and the words around it.
//!
//! The journal itself (types, restore, grouping) is `bookx_core::annotation_journal` and its
//! storage is `bookx_store::annotation_history`; this is the glue that needs the app: it applies a
//! restore to the library as one save with the source "undo" (so the restore is journalled and can
//! be undone), and it settles the account with the annotation sync ledger so that a device does not
//! delete the restored annotation again:
//!
//! * a highlight a **device deleted** was brought by that device (`made_here`) and linked in the
//!   ledger. Restoring it unlinks it: it is then an ordinary Bookx highlight, which the next sync
//!   sends to the device as a new annotation, and nothing a device sent earlier (its tombstone for
//!   the old annotation id) can remove it, because only a *linked* highlight is ever removed.
//! * a highlight a device **edited** (its note) stays linked; the ledger is told the restored note
//!   is the agreed one and the record is newer than what the device sent, so the device's older
//!   state does not win again. The restored text reaches the device when it is edited there; Bookx
//!   never pushes over a device's version of its own annotation.
//! * bookmarks a device deleted were never deleted here (a sync never deletes a bookmark), so only
//!   what the reader deleted needs restoring, and the ledger's tombstone keeps device copies from
//!   being imported twice.
//! * what the reader deleted in Bookx and sent to the device as a tombstone is sent again, newer.

use anyhow::Result;
use bookx_core::annotation_journal::{
    self as journal, Action, Entry, Group, Kind, Outcome, Record, Source, WriteContext,
};
use bookx_core::library::Library;
use bookx_store::SqliteStore;

use crate::annotation_sync::{Ledger, Report, State};
use crate::i18n::{ntr, tr, trf};

/// What a restore did.
#[derive(Debug, Default)]
pub struct Restored {
    /// Every requested entry with what happened to it.
    pub outcomes: Vec<(Entry, Outcome)>,
    /// History mark taken before the restore: [`undo`] reverts what was written after it.
    pub mark: u64,
}

impl Restored {
    /// Records that came back (restored or replaced).
    pub fn count(&self) -> usize {
        self.outcomes.iter().filter(|(_, o)| o.changed()).count()
    }

    /// Entries that could not be restored, with the reason in words.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (e, o) in &self.outcomes {
            let what = e.record.excerpt(40);
            match o {
                Outcome::SamePassage => out.push(trf(
                    "“{}” is already highlighted at that place, so it was not added again",
                    &[&what],
                )),
                Outcome::BookMissing => out.push(trf(
                    "“{}” belongs to “{}”, which is not in the library any more",
                    &[&what, &e.title],
                )),
                _ => {}
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// How many were already there.
    pub fn already(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, o)| *o == Outcome::AlreadyThere)
            .count()
    }
}

/// Who is responsible for each book a merge changed: the device that brought the change.
pub fn context_for(report: &Report) -> WriteContext {
    let mut ctx = WriteContext::by(Source::Device(tr("a device")));
    for d in &report.books {
        ctx.per_book.insert(
            d.book_id,
            Source::Device(crate::sync_activity::device_name(&d.device)),
        );
    }
    ctx
}

/// Settles the sync ledger for one restored entry (see the module comment).
pub fn settle_ledger(ledger: &mut Ledger, e: &Entry, now: i64) {
    let Record::Highlight(h) = &e.record else {
        return;
    };
    for inc in ledger.incoming.values_mut() {
        if inc.book_id != e.book || inc.highlight.as_deref() != Some(h.id.as_str()) {
            continue;
        }
        match (e.action, inc.state) {
            // a device deleted it: it belongs to the reader now and goes to the device as new
            (Action::Deleted, State::Removed) => {
                inc.highlight = None;
                inc.made_here = false;
            }
            // a device changed it: the restored state is the agreed one and newer than the device's
            (Action::Edited, State::Linked) => {
                inc.note = h.note.clone();
                inc.updated = inc.updated.max(now);
            }
            _ => {}
        }
    }
}

/// Puts the records of `ids` back into the library. One save, source "undo", so it is journalled;
/// the readers and the library views are told what changed.
pub fn restore(
    lib: &mut Library,
    store: &SqliteStore,
    ledger: &mut Ledger,
    ids: &[u64],
    now: i64,
) -> Result<Restored> {
    let entries = store.history_entries(ids)?;
    let mark = store.history_mark();
    let before = crate::sync_bus::Snapshot::take(&lib.books);
    let mut out = Restored {
        outcomes: Vec::new(),
        mark,
    };
    // edits first: a deleted record's place may be held by an edited twin that is going back
    let mut order: Vec<&Entry> = entries.iter().collect();
    order.sort_by_key(|e| e.action == Action::Deleted);
    for e in order {
        let o = journal::restore(e, lib.get_mut(e.book));
        if o.changed() {
            settle_ledger(ledger, e, now);
        }
        out.outcomes.push((e.clone(), o));
    }
    if out.count() > 0 {
        lib.with_source(Source::Undo, || lib.save())?;
        crate::sync_bus::publish_changes(&before, &lib.books, "", crate::sync_bus::Via::Import);
    }
    Ok(out)
}

/// Takes a restore back: the entries it wrote are reverted (restored records removed again,
/// replaced ones put back). Returns how many records changed.
pub fn undo(lib: &mut Library, store: &SqliteStore, mark: u64) -> Result<usize> {
    let written = store.history_since(mark)?;
    let before = crate::sync_bus::Snapshot::take(&lib.books);
    let mut n = 0;
    for e in written.iter().rev() {
        if e.source == Source::Undo && journal::revert(e, lib.get_mut(e.book)).changed() {
            n += 1;
        }
    }
    if n > 0 {
        lib.with_source(Source::Undo, || lib.save())?;
        crate::sync_bus::publish_changes(&before, &lib.books, "", crate::sync_bus::Via::Import);
    }
    Ok(n)
}

// -- words ------------------------------------------------------------------------------------

/// "3 highlights", "1 note", "2 bookmarks" (what is in `entries`).
pub fn counts_text(entries: &[Entry]) -> String {
    let n = |k: Kind| entries.iter().filter(|e| e.kind == k).count();
    let mut parts = Vec::new();
    for (k, singular, plural) in [
        (Kind::Highlight, "{} highlight", "{} highlights"),
        (Kind::Note, "{} note", "{} notes"),
        (Kind::Bookmark, "{} bookmark", "{} bookmarks"),
    ] {
        if n(k) > 0 {
            parts.push(ntr(singular, plural, n(k)));
        }
    }
    parts.join(", ")
}

/// Who, for a sentence: "you", "Kobo Libra".
pub fn who(s: &Source) -> String {
    match s {
        Source::You => tr("you"),
        Source::Device(d) => d.clone(),
        Source::Import => tr("an import"),
        Source::Merge => tr("a merge"),
        Source::Undo => tr("Undo"),
    }
}

/// "Deleted by Kobo Libra", "Edited by you".
pub fn what_happened(action: Action, s: &Source) -> String {
    match action {
        Action::Deleted => trf("Deleted by {}", &[&who(s)]),
        Action::Edited => trf("Edited by {}", &[&who(s)]),
        Action::Added => trf("Added by {}", &[&who(s)]),
    }
}

/// The heading of a group: "Kobo Libra removed 40 highlights from Dune".
pub fn group_title(g: &Group) -> String {
    let what = counts_text(&g.entries);
    match (&g.source, g.action) {
        (Source::Device(d), Action::Deleted) => trf("{} removed {} from {}", &[d, &what, &g.title]),
        (Source::Device(d), Action::Edited) => trf("{} changed {} in {}", &[d, &what, &g.title]),
        (Source::Device(d), Action::Added) => trf("{} added {} to {}", &[d, &what, &g.title]),
        (_, Action::Deleted) => trf("{} deleted from {}", &[&what, &g.title]),
        (_, Action::Edited) => trf("{} edited in {}", &[&what, &g.title]),
        (_, Action::Added) => trf("{} added to {}", &[&what, &g.title]),
    }
}

/// The toast after a sync took annotations out of the library: one sentence per book and device
/// (the first), or `None` when nothing was removed.
pub fn removed_toast(entries: &[Entry]) -> Option<(String, Vec<u64>)> {
    let gone: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.action == Action::Deleted && matches!(e.source, Source::Device(_)))
        .collect();
    let first = gone.first()?;
    let Source::Device(device) = &first.source else {
        return None;
    };
    let same: Vec<Entry> = gone
        .iter()
        .filter(|e| e.book == first.book && e.source == first.source)
        .map(|e| (*e).clone())
        .collect();
    let others = gone.len() - same.len();
    let mut text = trf(
        "{} removed {} from {}",
        &[device, &counts_text(&same), &first.title],
    );
    if others > 0 {
        text = trf("{} (and {} more in other books)", &[&text, &others]);
    }
    Some((text, gone.iter().map(|e| e.id).collect()))
}

/// The sentence for the result of a restore ("Restored 3 highlights").
pub fn restored_toast(r: &Restored) -> String {
    let done: Vec<Entry> = r
        .outcomes
        .iter()
        .filter(|(_, o)| o.changed())
        .map(|(e, _)| e.clone())
        .collect();
    if done.is_empty() {
        let skipped = r.problems();
        return match skipped.first() {
            Some(p) => p.clone(),
            None => tr("Nothing to restore: it is already in the book"),
        };
    }
    let mut s = trf("Restored {}", &[&counts_text(&done)]);
    let problems = r.problems().len() + r.already();
    if problems > 0 {
        s = trf(
            "{} ({} were skipped)",
            &[&s, &(r.outcomes.len() - done.len())],
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation_sync::Incoming;
    use crate::core::library::Highlight;
    use chrono::Utc;

    fn hl(id: &str, note: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start: 4,
            end: 9,
            color: "yellow".into(),
            style: "background".into(),
            note: note.into(),
            text: "quote".into(),
            created: Utc::now(),
        }
    }

    fn entry(action: Action, h: Highlight, device: &str) -> Entry {
        Entry {
            id: 1,
            at: Utc::now(),
            group: 1,
            book: 1,
            title: "Dune".into(),
            authors: "Frank".into(),
            kind: Record::Highlight(h.clone()).kind(),
            action,
            source: Source::Device(device.into()),
            record: Record::Highlight(h),
        }
    }

    fn incoming(state: State, made_here: bool) -> Incoming {
        Incoming {
            book_id: 1,
            highlight: Some("h1".into()),
            source: "KOReader (Kobo Libra)".into(),
            note: "device note".into(),
            device_look: Default::default(),
            updated: 100,
            seq: 5,
            state,
            text: "quote".into(),
            made_here,
            hint: Default::default(),
        }
    }

    #[test]
    fn a_restored_device_deletion_is_not_linked_to_the_device_any_more() {
        let mut l = Ledger::default();
        l.incoming
            .insert("doc/a".into(), incoming(State::Removed, true));
        settle_ledger(
            &mut l,
            &entry(Action::Deleted, hl("h1", ""), "Kobo Libra"),
            500,
        );
        let i = &l.incoming["doc/a"];
        assert!(i.highlight.is_none() && !i.made_here);
        assert_eq!(
            i.state,
            State::Removed,
            "the device's tombstone is still remembered"
        );
    }

    #[test]
    fn a_restored_device_edit_is_newer_than_the_devices_record() {
        let mut l = Ledger::default();
        l.incoming
            .insert("doc/a".into(), incoming(State::Linked, true));
        settle_ledger(
            &mut l,
            &entry(Action::Edited, hl("h1", "my note"), "Kobo Libra"),
            500,
        );
        let i = &l.incoming["doc/a"];
        assert_eq!(
            (i.note.as_str(), i.updated, i.state),
            ("my note", 500, State::Linked)
        );
    }

    #[test]
    fn other_highlights_and_books_are_left_alone() {
        let mut l = Ledger::default();
        l.incoming
            .insert("doc/a".into(), incoming(State::Removed, true));
        let before = l.clone();
        settle_ledger(
            &mut l,
            &entry(Action::Deleted, hl("other", ""), "Kobo"),
            500,
        );
        assert_eq!(l, before);
        let mut e = entry(Action::Deleted, hl("h1", ""), "Kobo");
        e.book = 2;
        settle_ledger(&mut l, &e, 500);
        assert_eq!(l, before);
    }

    #[test]
    fn context_names_the_device_per_book() {
        let report = Report {
            books: vec![crate::annotation_sync::BookDelta {
                book_id: 4,
                device: "Kobo Libra".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let c = context_for(&report);
        assert_eq!(c.source_for(4), &Source::Device("Kobo Libra".into()));
        assert!(matches!(c.source_for(9), Source::Device(_)));
    }

    #[test]
    fn the_toast_names_device_count_and_book() {
        let es: Vec<Entry> = (0..3)
            .map(|i| {
                let mut e = entry(Action::Deleted, hl(&format!("h{i}"), ""), "Kobo Libra");
                e.id = i + 1;
                e
            })
            .collect();
        let (text, ids) = removed_toast(&es).unwrap();
        assert_eq!(text, "Kobo Libra removed 3 highlights from Dune");
        assert_eq!(ids, vec![1, 2, 3]);
        assert!(removed_toast(&[]).is_none());
        let mine = Entry {
            source: Source::You,
            ..es[0].clone()
        };
        assert!(
            removed_toast(&[mine]).is_none(),
            "your own deletions need no toast"
        );
        let mut more = es.clone();
        more.push(Entry {
            book: 2,
            title: "Emma".into(),
            ..es[0].clone()
        });
        assert!(removed_toast(&more).unwrap().0.contains("1 more"));
    }

    #[test]
    fn rows_say_who_and_what() {
        assert_eq!(
            what_happened(Action::Deleted, &Source::Device("Kobo Libra".into())),
            "Deleted by Kobo Libra"
        );
        assert_eq!(what_happened(Action::Edited, &Source::You), "Edited by you");
    }

    #[test]
    fn counts_mix_kinds() {
        let a = entry(Action::Deleted, hl("a", ""), "K");
        let b = entry(Action::Deleted, hl("b", "n"), "K");
        assert_eq!(counts_text(&[a.clone(), b]), "1 highlight, 1 note");
        assert_eq!(counts_text(&[a]), "1 highlight");
    }
}
