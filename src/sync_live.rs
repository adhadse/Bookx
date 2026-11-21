//! What an open reader does when a sync changes its book (see `sync_bus`): which highlights to draw
//! or take away, when a note being edited has been changed on the e-reader, and the little banner
//! ("Your Kobo Libra is at 62%") with its rules. Pure functions and plain data, no GTK; the reader
//! (`ui/reader.rs`, `ui/reader_live.rs`) only applies the answers.

use std::collections::{HashMap, HashSet};

use crate::core::library::Book;
use crate::i18n::{tr, trf};
use crate::kosync_sync::ASK_GAP;

/// A banner stays this long (milliseconds) unless the reader turns a page or answers it.
pub const BANNER_MS: u64 = 15_000;

// -- highlights -------------------------------------------------------------------------------------

/// What the page needs to know about a highlight to draw it.
#[derive(Debug, Clone, PartialEq)]
pub struct HlSig {
    pub id: String,
    pub spine: usize,
    pub start: i32,
    pub end: i32,
    pub color: String,
    pub style: String,
    pub note: String,
}

/// The highlights of a book as the text reader paints them (no PDF highlights, no unplaced ones).
pub fn sigs_of(book: &Book) -> Vec<HlSig> {
    book.highlights
        .iter()
        .filter(|h| !crate::pdf::is_pdf(h) && !crate::core::anchor::is_unanchored(h))
        .map(|h| HlSig {
            id: h.id.clone(),
            spine: h.spine,
            start: h.start,
            end: h.end,
            color: h.color.clone(),
            style: h.style.clone(),
            note: h.note.clone(),
        })
        .collect()
}

/// What changed between two lists of highlights, by id (each id is in one list only).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HlDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// The place changed (the highlight was anchored again).
    pub moved: Vec<String>,
    pub recoloured: Vec<String>,
    pub restyled: Vec<String>,
    pub renoted: Vec<String>,
}

pub fn diff(before: &[HlSig], after: &[HlSig]) -> HlDiff {
    let old: HashMap<&str, &HlSig> = before.iter().map(|s| (s.id.as_str(), s)).collect();
    let new: HashSet<&str> = after.iter().map(|s| s.id.as_str()).collect();
    let mut d = HlDiff::default();
    for s in after {
        match old.get(s.id.as_str()) {
            None => d.added.push(s.id.clone()),
            Some(o) => {
                if (o.spine, o.start, o.end) != (s.spine, s.start, s.end) {
                    d.moved.push(s.id.clone());
                }
                if o.color != s.color {
                    d.recoloured.push(s.id.clone());
                }
                if o.style != s.style {
                    d.restyled.push(s.id.clone());
                }
                if o.note != s.note {
                    d.renoted.push(s.id.clone());
                }
            }
        }
    }
    for s in before {
        if !new.contains(s.id.as_str()) {
            d.removed.push(s.id.clone());
        }
    }
    d
}

impl HlDiff {
    pub fn is_empty(&self) -> bool {
        *self == HlDiff::default()
    }

    /// Highlights to draw (new, or drawn differently now), each once, in the order of `after`.
    pub fn draw(&self, after: &[HlSig]) -> Vec<String> {
        let want: HashSet<&str> = self
            .added
            .iter()
            .chain(&self.moved)
            .chain(&self.recoloured)
            .chain(&self.restyled)
            .chain(&self.renoted)
            .map(String::as_str)
            .collect();
        after
            .iter()
            .filter(|s| want.contains(s.id.as_str()))
            .map(|s| s.id.clone())
            .collect()
    }

    /// The page of chapter `spine` must be painted again: something that touches it changed
    /// (a note alone changes the marker the page draws, so it counts).
    pub fn repaints(&self, before: &[HlSig], after: &[HlSig], spine: usize) -> bool {
        let here: HashSet<&str> = before
            .iter()
            .chain(after)
            .filter(|s| s.spine == spine)
            .map(|s| s.id.as_str())
            .collect();
        self.added
            .iter()
            .chain(&self.removed)
            .chain(&self.moved)
            .chain(&self.recoloured)
            .chain(&self.restyled)
            .chain(&self.renoted)
            .any(|id| here.contains(id.as_str()))
    }
}

// -- a note being edited ------------------------------------------------------------------------------

/// The note the reader is editing now: which highlight, and the note as it was when the editor opened.
#[derive(Debug, Clone, PartialEq)]
pub struct Editing {
    pub id: String,
    pub original: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NoteAction {
    /// Nothing to do.
    Nothing,
    /// The editor holds the text it opened with: show the new note in it, say nothing.
    ReplaceQuietly(String),
    /// The reader typed something and the note changed on the device: keep the editor, ask.
    Ask { theirs: String },
    /// The highlight was deleted on the device while it is edited: keep the editor, tell.
    Gone,
}

/// What to do with an open note editor after a sync. `now` is the note in the library (`None`:
/// the highlight is gone), `typed` what the editor holds.
pub fn note_action(editing: Option<&Editing>, now: Option<&str>, typed: &str) -> NoteAction {
    let Some(e) = editing else {
        return NoteAction::Nothing;
    };
    match now {
        None => NoteAction::Gone,
        Some(n) if n == e.original => NoteAction::Nothing,
        Some(n) if typed == e.original => NoteAction::ReplaceQuietly(n.to_string()),
        Some(n) if n == typed => NoteAction::Nothing,
        Some(n) => NoteAction::Ask {
            theirs: n.to_string(),
        },
    }
}

// -- the banner -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Banner {
    /// A device is further along than this reader.
    Ahead {
        device: String,
        percentage: f64,
        page: Option<usize>,
    },
    /// The note being edited changed on the e-reader.
    NoteChanged { id: String, theirs: String },
    /// The highlight being edited was deleted on the e-reader.
    NoteGone { id: String },
}

impl Banner {
    pub fn text(&self) -> String {
        match self {
            Banner::Ahead {
                device,
                percentage,
                page,
            } => {
                let name = if device.trim().is_empty() {
                    tr("another device")
                } else {
                    device.clone()
                };
                let pct = format!("{:.0}", percentage * 100.0);
                match page {
                    Some(p) => trf(
                        "Your {} is at {}% (page {})",
                        &[&name, &pct, &p.to_string()],
                    ),
                    None => trf("Your {} is at {}%", &[&name, &pct]),
                }
            }
            Banner::NoteChanged { .. } => tr("This note changed on your e-reader"),
            Banner::NoteGone { .. } => tr("This highlight was removed on your e-reader"),
        }
    }

    /// The labels of the buttons, the first is the suggested one.
    pub fn buttons(&self) -> Vec<String> {
        match self {
            Banner::Ahead { .. } => vec![tr("Continue There"), tr("Dismiss")],
            Banner::NoteChanged { .. } => vec![tr("Use Theirs"), tr("Keep Mine")],
            Banner::NoteGone { .. } => vec![tr("Dismiss")],
        }
    }

    fn timed(&self) -> bool {
        matches!(self, Banner::Ahead { .. })
    }
}

/// The page of the book a position falls on, from the share of the book and the page count.
pub fn page_at(percentage: f64, total_pages: usize) -> Option<usize> {
    (total_pages > 0).then(|| {
        (((percentage.clamp(0.0, 1.0) * total_pages as f64).floor() as usize) + 1).min(total_pages)
    })
}

/// Whether a position reported by a device is worth a banner: the offer is on, it comes from
/// another device and is more than one per cent ahead of the reader.
pub fn worth_offering(enabled: bool, local: f64, ahead: f64, same_device: bool) -> bool {
    enabled && !same_device && ahead > local + ASK_GAP
}

/// The banner of one reader: at most one at a time, a newer position of the same device replaces
/// the older, one that was dismissed is not offered again, it goes away by itself after
/// [`BANNER_MS`] and when the reader turns a page. Times are milliseconds from a steady clock.
#[derive(Debug, Default)]
pub struct Live {
    current: Option<(Banner, u64)>,
    dismissed: HashSet<(String, u32)>,
}

fn key(device: &str, pct: f64) -> (String, u32) {
    (device.to_string(), (pct * 100.0).round() as u32)
}

impl Live {
    pub fn current(&self) -> Option<&Banner> {
        self.current.as_ref().map(|(b, _)| b)
    }

    /// Offers a position. Returns whether the banner on screen changed.
    pub fn offer_ahead(&mut self, device: &str, pct: f64, page: Option<usize>, now: u64) -> bool {
        if self.dismissed.contains(&key(device, pct)) {
            return false;
        }
        let b = Banner::Ahead {
            device: device.to_string(),
            percentage: pct,
            page,
        };
        match &self.current {
            // a question about a note is not pushed away by a position
            Some((Banner::NoteChanged { .. } | Banner::NoteGone { .. }, _)) => false,
            Some((c, _)) if *c == b => false,
            _ => {
                self.current = Some((b, now));
                true
            }
        }
    }

    /// Shows the note question (it stays until it is answered or the editor closes).
    pub fn offer_note(&mut self, b: Banner, now: u64) -> bool {
        if self.current.as_ref().is_some_and(|(c, _)| *c == b) {
            return false;
        }
        self.current = Some((b, now));
        true
    }

    /// The reader answered or closed it. A dismissed position is not offered again.
    pub fn dismiss(&mut self) -> bool {
        match self.current.take() {
            Some((
                Banner::Ahead {
                    device, percentage, ..
                },
                _,
            )) => {
                self.dismissed.insert(key(&device, percentage));
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    /// The reader turned a page: a position goes away, a note question stays.
    pub fn page_turned(&mut self) -> bool {
        if self.current.as_ref().is_some_and(|(b, _)| b.timed()) {
            return self.dismiss();
        }
        false
    }

    /// Takes a banner away that has been there long enough. Returns whether it did.
    pub fn tick(&mut self, now: u64) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|(b, at)| b.timed() && now >= at + BANNER_MS)
        {
            return self.dismiss();
        }
        false
    }

    /// The note editor closed: its question is moot.
    pub fn editor_closed(&mut self) -> bool {
        if self.current.as_ref().is_some_and(|(b, _)| !b.timed()) {
            self.current = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, spine: usize, color: &str, note: &str) -> HlSig {
        HlSig {
            id: id.into(),
            spine,
            start: 10,
            end: 20,
            color: color.into(),
            style: String::new(),
            note: note.into(),
        }
    }

    #[test]
    fn the_diff_lists_what_to_draw_and_what_to_take_away() {
        let before = vec![
            s("a", 0, "yellow", ""),
            s("b", 0, "green", ""),
            s("c", 1, "blue", ""),
            s("d", 1, "red", ""),
        ];
        let mut moved = s("d", 1, "red", "");
        moved.start = 99;
        let after = vec![
            s("a", 0, "purple", ""),
            s("b", 0, "green", "new note"),
            s("e", 0, "pink", ""),
            moved,
        ];
        let d = diff(&before, &after);
        assert_eq!(d.added, vec!["e"]);
        assert_eq!(d.removed, vec!["c"]);
        assert_eq!(d.recoloured, vec!["a"]);
        assert_eq!(d.renoted, vec!["b"]);
        assert_eq!(d.moved, vec!["d"]);
        assert_eq!(d.draw(&after), vec!["a", "b", "e", "d"]);
        assert!(!d.is_empty());
    }

    #[test]
    fn nothing_changed_means_nothing_to_paint() {
        let a = vec![s("a", 0, "yellow", "x"), s("b", 1, "red", "")];
        let d = diff(&a, &a.clone());
        assert!(d.is_empty());
        assert!(d.draw(&a).is_empty());
        assert!(!d.repaints(&a, &a, 0));
    }

    #[test]
    fn only_a_change_in_the_chapter_on_screen_repaints_it() {
        let before = vec![s("a", 0, "yellow", ""), s("c", 2, "blue", "")];
        let after = vec![s("a", 0, "yellow", ""), s("c", 2, "red", "")];
        let d = diff(&before, &after);
        assert!(d.repaints(&before, &after, 2));
        assert!(!d.repaints(&before, &after, 0));
        // a highlight that vanished from this chapter is a repaint too
        let gone: Vec<HlSig> = before.iter().filter(|x| x.id != "a").cloned().collect();
        assert!(diff(&before, &gone).repaints(&before, &gone, 0));
    }

    #[test]
    fn a_note_open_in_the_editor_is_never_replaced_behind_the_back_of_the_reader() {
        let e = Editing {
            id: "a".into(),
            original: "mine".into(),
        };
        assert_eq!(note_action(None, Some("x"), ""), NoteAction::Nothing);
        assert_eq!(
            note_action(Some(&e), Some("mine"), "mine and more"),
            NoteAction::Nothing
        );
        assert_eq!(
            note_action(Some(&e), Some("theirs"), "mine"),
            NoteAction::ReplaceQuietly("theirs".into()),
            "nothing typed yet: just show the new note"
        );
        assert_eq!(
            note_action(Some(&e), Some("theirs"), "mine and more"),
            NoteAction::Ask {
                theirs: "theirs".into()
            }
        );
        assert_eq!(
            note_action(Some(&e), Some("same"), "same"),
            NoteAction::Nothing,
            "both sides wrote the same words"
        );
        assert_eq!(note_action(Some(&e), None, "typed"), NoteAction::Gone);
    }

    #[test]
    fn a_position_is_offered_only_when_a_device_is_clearly_ahead() {
        assert!(worth_offering(true, 0.30, 0.62, false));
        assert!(
            !worth_offering(false, 0.30, 0.62, false),
            "the switch is off"
        );
        assert!(
            !worth_offering(true, 0.30, 0.305, false),
            "less than one per cent"
        );
        assert!(
            !worth_offering(true, 0.70, 0.62, false),
            "behind is never offered"
        );
        assert!(!worth_offering(true, 0.30, 0.62, true), "this very device");
    }

    #[test]
    fn pages_are_one_based_and_clamped() {
        assert_eq!(page_at(0.0, 300), Some(1));
        assert_eq!(page_at(0.62, 300), Some(187));
        assert_eq!(page_at(1.0, 300), Some(300));
        assert_eq!(page_at(0.5, 0), None);
    }

    #[test]
    fn the_banner_text_names_the_device_and_the_place() {
        let b = Banner::Ahead {
            device: "Kobo Libra".into(),
            percentage: 0.62,
            page: Some(143),
        };
        assert_eq!(b.text(), "Your Kobo Libra is at 62% (page 143)");
        assert_eq!(b.buttons(), vec!["Continue There", "Dismiss"]);
        let n = Banner::Ahead {
            device: " ".into(),
            percentage: 0.5,
            page: None,
        };
        assert_eq!(n.text(), "Your another device is at 50%");
        let c = Banner::NoteChanged {
            id: "a".into(),
            theirs: "x".into(),
        };
        assert_eq!(c.buttons(), vec!["Use Theirs", "Keep Mine"]);
        assert_eq!(c.text(), "This note changed on your e-reader");
    }

    #[test]
    fn one_banner_per_device_and_a_newer_position_replaces_the_older() {
        let mut l = Live::default();
        assert!(l.offer_ahead("Kobo", 0.5, None, 0));
        assert!(
            !l.offer_ahead("Kobo", 0.5, None, 100),
            "the same position is one banner"
        );
        assert!(l.offer_ahead("Kobo", 0.6, Some(10), 200));
        assert!(
            matches!(l.current(), Some(Banner::Ahead { percentage, .. }) if *percentage == 0.6)
        );
    }

    #[test]
    fn a_dismissed_position_is_not_offered_again_but_a_new_one_is() {
        let mut l = Live::default();
        l.offer_ahead("Kobo", 0.5, None, 0);
        assert!(l.dismiss());
        assert!(l.current().is_none());
        assert!(!l.offer_ahead("Kobo", 0.5, None, 10));
        assert!(l.offer_ahead("Kobo", 0.55, None, 20));
    }

    #[test]
    fn a_banner_goes_after_fifteen_seconds_or_a_page_turn() {
        let mut l = Live::default();
        l.offer_ahead("Kobo", 0.5, None, 1000);
        assert!(!l.tick(1000 + BANNER_MS - 1));
        assert!(l.tick(1000 + BANNER_MS));
        assert!(l.current().is_none());
        l.offer_ahead("Kobo", 0.7, None, 0);
        assert!(l.page_turned());
        assert!(l.current().is_none());
        assert!(!l.page_turned());
    }

    #[test]
    fn a_note_question_outlives_the_clock_and_page_turns_and_a_position_does_not_push_it_away() {
        let mut l = Live::default();
        let q = Banner::NoteChanged {
            id: "a".into(),
            theirs: "t".into(),
        };
        assert!(l.offer_note(q.clone(), 0));
        assert!(!l.offer_note(q.clone(), 5));
        assert!(!l.offer_ahead("Kobo", 0.9, None, 10));
        assert!(!l.tick(10 * BANNER_MS));
        assert!(!l.page_turned());
        assert_eq!(l.current(), Some(&q));
        assert!(l.editor_closed());
        assert!(l.current().is_none());
        // closing the editor leaves a position alone
        l.offer_ahead("Kobo", 0.9, None, 0);
        assert!(!l.editor_closed());
        assert!(l.current().is_some());
    }
}
