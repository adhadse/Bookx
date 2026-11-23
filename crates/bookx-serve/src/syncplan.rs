//! The "progressive, conserving" sync between a device and the library (enhancements item 4).
//!
//! Pure functions, no I/O. Given what each side knows about its books, [`plan`] works out what
//! would flow from the device to the library and from the library to the device. The rules:
//!
//! * Nothing is ever deleted or lowered. A plan holds only additions and "move forward" steps, and
//!   there is no variant that removes anything, so a bug cannot make a plan destructive.
//! * Reading progress moves to the side that is behind, never the other way. *Finished* counts as
//!   100 % and is never undone. When the side that is behind was read more recently, nothing moves
//!   and the difference is reported (the reader may have started the book again on purpose).
//! * A highlight or note that exists on one side only is added to the other. One that exists on
//!   both sides is the same highlight; if one copy has more (a note, or fuller text), the richer
//!   copy flows to the poorer. If both have a different note, both are kept and it is reported as
//!   a conflict for the user to decide.
//! * Books are paired by title (with the authors as tie-break); a book that cannot be paired
//!   unambiguously is left out and reported.
//!
//! Applying a plan is the caller's business; applying it and planning again gives an empty plan.

use crate::reading::Status;

/// One highlight or note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annot {
    pub text: String,
    pub note: String,
    /// Any time stamp the side keeps, shown to the user only.
    pub when: Option<String>,
}

/// What one side knows about one book.
#[derive(Debug, Clone, PartialEq)]
pub struct SideBook {
    /// Opaque id the caller uses to find the book again (library id, device path).
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    /// 0.0 to 1.0
    pub progress: f64,
    pub status: Status,
    /// Seconds since the epoch.
    pub last_read: Option<i64>,
    pub notes: Vec<Annot>,
}

impl SideBook {
    /// Progress with *Finished* counted as complete.
    pub fn effective_progress(&self) -> f64 {
        if self.status == Status::Finished {
            1.0
        } else {
            self.progress.clamp(0.0, 1.0)
        }
    }
}

/// Which side a change goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Library,
    Device,
}

/// What happens to the reading progress of a book.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    InSync,
    /// The side named is behind and moves forward to `to`.
    Forward {
        to_side: Side,
        from: f64,
        to: f64,
        finished: bool,
    },
}

/// A note that exists with more information on one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enrich {
    pub to_side: Side,
    pub annot: Annot,
}

/// Both sides have the same highlight with different notes: nothing moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub text: String,
    pub library_note: String,
    pub device_note: String,
}

/// The plan for one paired book.
#[derive(Debug, Clone, PartialEq)]
pub struct BookPlan {
    pub library_id: String,
    pub device_id: String,
    pub title: String,
    pub progress: Progress,
    /// The newer reading time stamp goes to the side that has none or an older one.
    pub last_read_to: Option<(Side, i64)>,
    /// Highlights only the device has: added to the library.
    pub notes_to_library: Vec<Annot>,
    /// Highlights only the library has: added to the device.
    pub notes_to_device: Vec<Annot>,
    pub enrich: Vec<Enrich>,
    pub conflicts: Vec<Conflict>,
    /// The side that is behind on progress was read more recently; nothing was moved because of it.
    pub behind_but_newer: Option<Side>,
}

impl BookPlan {
    pub fn is_empty(&self) -> bool {
        self.progress == Progress::InSync
            && self.last_read_to.is_none()
            && self.notes_to_library.is_empty()
            && self.notes_to_device.is_empty()
            && self.enrich.is_empty()
    }
}

/// Why a book is not in the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// A device book with no library counterpart (not an error: the library may not have it).
    NotInLibrary { id: String, title: String },
    /// More than one library book fits and the authors do not decide.
    Ambiguous { id: String, title: String },
}

/// Totals for the "this is what is going to happen" message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub books_paired: usize,
    pub in_sync: usize,
    pub progress_to_library: usize,
    pub progress_to_device: usize,
    pub notes_to_library: usize,
    pub notes_to_device: usize,
    pub enriched_library: usize,
    pub enriched_device: usize,
    pub conflicts: usize,
    pub behind_but_newer: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SyncPlan {
    pub books: Vec<BookPlan>,
    pub skipped: Vec<Skipped>,
    /// Library books with no counterpart on the device (informational).
    pub library_only: usize,
}

impl SyncPlan {
    /// Books where something happens.
    pub fn changes(&self) -> impl Iterator<Item = &BookPlan> {
        self.books.iter().filter(|b| !b.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.changes().next().is_none()
    }

    pub fn totals(&self) -> Totals {
        let mut t = Totals {
            books_paired: self.books.len(),
            skipped: self.skipped.len(),
            ..Default::default()
        };
        for b in &self.books {
            match b.progress {
                Progress::InSync => t.in_sync += 1,
                Progress::Forward {
                    to_side: Side::Library,
                    ..
                } => t.progress_to_library += 1,
                Progress::Forward {
                    to_side: Side::Device,
                    ..
                } => t.progress_to_device += 1,
            }
            t.notes_to_library += b.notes_to_library.len();
            t.notes_to_device += b.notes_to_device.len();
            for e in &b.enrich {
                match e.to_side {
                    Side::Library => t.enriched_library += 1,
                    Side::Device => t.enriched_device += 1,
                }
            }
            t.conflicts += b.conflicts.len();
            t.behind_but_newer += b.behind_but_newer.is_some() as usize;
        }
        t
    }

    /// One sentence saying what flows where, for the prompt shown before syncing.
    pub fn describe(&self) -> String {
        let t = self.totals();
        if self.is_empty() && t.conflicts == 0 {
            return "Device and library are in sync. Nothing will be changed.".to_string();
        }
        let mut parts = Vec::new();
        let mut line = |n: usize, what: &str, dir: &str| {
            if n > 0 {
                parts.push(format!("{n} {what} {dir}"));
            }
        };
        line(
            t.progress_to_library,
            "reading position(s)",
            "from the device to the library",
        );
        line(
            t.progress_to_device,
            "reading position(s)",
            "from the library to the device",
        );
        line(
            t.notes_to_library,
            "highlight(s)/note(s)",
            "from the device to the library",
        );
        line(
            t.notes_to_device,
            "highlight(s)/note(s)",
            "from the library to the device",
        );
        line(t.enriched_library, "note(s) completed", "in the library");
        line(t.enriched_device, "note(s) completed", "on the device");
        let mut s = format!(
            "{}. Nothing is removed or overwritten on either side.",
            parts.join(", ")
        );
        if t.conflicts > 0 {
            s.push_str(&format!(
                " {} highlight(s) have different notes on the two sides; both are kept.",
                t.conflicts
            ));
        }
        s
    }
}

// -- matching -------------------------------------------------------------------------------------

/// Lower-case letters and digits only: `The Road: A Novel` and `the road a novel` agree.
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn title_matches(device_title: &str, library_title: &str) -> bool {
    let (d, l) = (norm(device_title), norm(library_title));
    !l.is_empty() && (d == l || (l.len() >= 4 && d.ends_with(&l)))
}

/// One author as a key that does not care about the order of the names: `Newman, Sam` and
/// `Sam Newman` are the same person (the library keeps the first form, a device import gives the second).
fn author_key(s: &str) -> String {
    let mut words: Vec<String> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(norm)
        .filter(|w| !w.is_empty())
        .collect();
    words.sort();
    words.join(" ")
}

fn authors_overlap(a: &[String], b: &[String]) -> Option<bool> {
    let (a, b): (Vec<_>, Vec<_>) = (
        a.iter().map(|s| author_key(s)).collect(),
        b.iter().map(|s| author_key(s)).collect(),
    );
    if a.iter().all(|s| s.is_empty()) || b.iter().all(|s| s.is_empty()) {
        return None;
    }
    Some(a.iter().any(|x| !x.is_empty() && b.contains(x)))
}

// -- notes ----------------------------------------------------------------------------------------

/// Whether two annotations are the same highlight (text equal ignoring case and punctuation, or
/// one contained in the other when it is at least 12 characters long).
pub fn same_highlight(a: &Annot, b: &Annot) -> bool {
    let (x, y) = (norm(&a.text), norm(&b.text));
    if x.is_empty() || y.is_empty() {
        // notes without highlighted text: compare the notes
        return !norm(&a.note).is_empty() && norm(&a.note) == norm(&b.note) && x == y;
    }
    if x == y {
        return true;
    }
    let (short, long) = if x.len() <= y.len() {
        (&x, &y)
    } else {
        (&y, &x)
    };
    short.chars().count() >= 12 && long.contains(short.as_str())
}

fn richer_text<'a>(a: &'a str, b: &'a str) -> &'a str {
    if norm(a).len() >= norm(b).len() { a } else { b }
}

fn plan_notes(plan: &mut BookPlan, library: &[Annot], device: &[Annot]) {
    let mut used_lib = vec![false; library.len()];
    for d in device {
        if norm(&d.text).is_empty() && norm(&d.note).is_empty() {
            continue;
        }
        let found = library
            .iter()
            .enumerate()
            .find(|(i, l)| !used_lib[*i] && same_highlight(l, d));
        match found {
            None => {
                if !plan.notes_to_library.iter().any(|a| same_highlight(a, d)) {
                    plan.notes_to_library.push(d.clone());
                }
            }
            Some((i, l)) => {
                used_lib[i] = true;
                let text = richer_text(&l.text, &d.text).to_string();
                let (ln, dn) = (norm(&l.note), norm(&d.note));
                let note_for_both: Option<String> = if ln == dn {
                    Some(l.note.clone().max(d.note.clone()))
                } else if ln.is_empty() {
                    Some(d.note.clone())
                } else if dn.is_empty() || ln.contains(&dn) {
                    Some(l.note.clone())
                } else if dn.contains(&ln) {
                    Some(d.note.clone())
                } else {
                    plan.conflicts.push(Conflict {
                        text: text.clone(),
                        library_note: l.note.clone(),
                        device_note: d.note.clone(),
                    });
                    None
                };
                // text: a side with the shorter text gets the longer one
                for (side, have, other_text) in
                    [(Side::Library, l, &d.text), (Side::Device, d, &l.text)]
                {
                    let text_poorer = norm(&have.text).len() < norm(other_text).len();
                    let note_poorer = match &note_for_both {
                        Some(n) => norm(&have.note).len() < norm(n).len(),
                        None => false,
                    };
                    if text_poorer || note_poorer {
                        plan.enrich.push(Enrich {
                            to_side: side,
                            annot: Annot {
                                text: text.clone(),
                                note: note_for_both.clone().unwrap_or_else(|| have.note.clone()),
                                when: have.when.clone(),
                            },
                        });
                    }
                }
            }
        }
    }
    for (i, l) in library.iter().enumerate() {
        if used_lib[i] || (norm(&l.text).is_empty() && norm(&l.note).is_empty()) {
            continue;
        }
        if !plan.notes_to_device.iter().any(|a| same_highlight(a, l)) {
            plan.notes_to_device.push(l.clone());
        }
    }
}

// -- the plan ---------------------------------------------------------------------------------

const EPS: f64 = 0.005;
/// A side is "read more recently" when its time stamp is more than a day ahead.
const RECENT_GAP_SECS: i64 = 24 * 3600;

fn plan_book(lib: &SideBook, dev: &SideBook) -> BookPlan {
    let (pl, pd) = (lib.effective_progress(), dev.effective_progress());
    let mut plan = BookPlan {
        library_id: lib.id.clone(),
        device_id: dev.id.clone(),
        title: lib.title.clone(),
        progress: Progress::InSync,
        last_read_to: None,
        notes_to_library: Vec::new(),
        notes_to_device: Vec::new(),
        enrich: Vec::new(),
        conflicts: Vec::new(),
        behind_but_newer: None,
    };
    if pd > pl + EPS {
        plan.progress = Progress::Forward {
            to_side: Side::Library,
            from: pl,
            to: pd,
            finished: dev.status == Status::Finished,
        };
        if matches!((lib.last_read, dev.last_read), (Some(l), Some(d)) if l > d + RECENT_GAP_SECS) {
            plan.behind_but_newer = Some(Side::Device);
        }
    } else if pl > pd + EPS {
        plan.progress = Progress::Forward {
            to_side: Side::Device,
            from: pd,
            to: pl,
            finished: lib.status == Status::Finished,
        };
        if matches!((lib.last_read, dev.last_read), (Some(l), Some(d)) if d > l + RECENT_GAP_SECS) {
            plan.behind_but_newer = Some(Side::Library);
        }
    }
    // a side that is behind but was read more recently restarted the book on purpose: the other
    // side's forward step is withheld
    if plan.behind_but_newer.is_some() {
        plan.progress = Progress::InSync;
    }
    match (lib.last_read, dev.last_read) {
        (Some(l), Some(d)) if l > d => plan.last_read_to = Some((Side::Device, l)),
        (Some(l), Some(d)) if d > l => plan.last_read_to = Some((Side::Library, d)),
        (Some(l), None) => plan.last_read_to = Some((Side::Device, l)),
        (None, Some(d)) => plan.last_read_to = Some((Side::Library, d)),
        _ => {}
    }
    if plan.behind_but_newer.is_some() {
        // the time stamps tell the story of the restart; leave them alone too
        plan.last_read_to = None;
    }
    plan_notes(&mut plan, &lib.notes, &dev.notes);
    plan
}

/// How a title (with authors, when known) fits the books of a library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pairing {
    /// Index into the library slice.
    One(usize),
    /// More than one book fits and the authors do not decide.
    Ambiguous,
    /// No book fits (or the only one has clearly different authors).
    NoBook,
}

/// Finds the library book a foreign title belongs to: by title (case, punctuation and a leading
/// prefix such as `01 - ` ignored), with the authors as tie-break and as a veto.
pub fn pair(library: &[SideBook], title: &str, authors: &[String]) -> Pairing {
    let candidates: Vec<usize> = library
        .iter()
        .enumerate()
        .filter(|(_, l)| title_matches(title, &l.title))
        .map(|(i, _)| i)
        .collect();
    let pick = match candidates.len() {
        0 => return Pairing::NoBook,
        1 => candidates[0],
        _ => {
            let by_author: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|i| authors_overlap(&library[*i].authors, authors) == Some(true))
                .collect();
            if by_author.len() == 1 {
                by_author[0]
            } else {
                return Pairing::Ambiguous;
            }
        }
    };
    if authors_overlap(&library[pick].authors, authors) == Some(false) && candidates.len() == 1 {
        // same title, clearly different authors: not the same book
        return Pairing::NoBook;
    }
    Pairing::One(pick)
}

/// Plans the sync of `device` books against `library` books. See the module docs for the rules.
pub fn plan(library: &[SideBook], device: &[SideBook]) -> SyncPlan {
    let mut out = SyncPlan::default();
    let mut paired_lib = vec![false; library.len()];
    for d in device {
        let pick = match pair(library, &d.title, &d.authors) {
            Pairing::One(i) => i,
            Pairing::NoBook => {
                out.skipped.push(Skipped::NotInLibrary {
                    id: d.id.clone(),
                    title: d.title.clone(),
                });
                continue;
            }
            Pairing::Ambiguous => {
                out.skipped.push(Skipped::Ambiguous {
                    id: d.id.clone(),
                    title: d.title.clone(),
                });
                continue;
            }
        };
        paired_lib[pick] = true;
        out.books.push(plan_book(&library[pick], d));
    }
    out.library_only = paired_lib.iter().filter(|p| !**p).count();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(text: &str, note: &str) -> Annot {
        Annot {
            text: text.into(),
            note: note.into(),
            when: None,
        }
    }

    fn book(id: &str, title: &str, progress: f64, notes: Vec<Annot>) -> SideBook {
        SideBook {
            id: id.into(),
            title: title.into(),
            authors: vec![],
            progress,
            status: if progress >= 1.0 {
                Status::Finished
            } else if progress > 0.0 {
                Status::Reading
            } else {
                Status::Unread
            },
            last_read: None,
            notes,
        }
    }

    /// Applies a plan to both sides the way the app would (additions and forward steps only).
    fn apply(plan: &SyncPlan, lib: &mut [SideBook], dev: &mut [SideBook]) {
        for b in &plan.books {
            let l = lib.iter_mut().find(|x| x.id == b.library_id).unwrap();
            let d = dev.iter_mut().find(|x| x.id == b.device_id).unwrap();
            if let Progress::Forward {
                to_side,
                to,
                finished,
                ..
            } = b.progress
            {
                let t = if to_side == Side::Library {
                    &mut *l
                } else {
                    &mut *d
                };
                t.progress = to;
                if finished {
                    t.status = Status::Finished;
                } else if t.status == Status::Unread {
                    t.status = Status::Reading;
                }
            }
            if let Some((side, when)) = b.last_read_to {
                (if side == Side::Library {
                    &mut *l
                } else {
                    &mut *d
                })
                .last_read = Some(when);
            }
            l.notes.extend(b.notes_to_library.iter().cloned());
            d.notes.extend(b.notes_to_device.iter().cloned());
            for e in &b.enrich {
                let t = if e.to_side == Side::Library {
                    &mut *l
                } else {
                    &mut *d
                };
                if let Some(n) = t.notes.iter_mut().find(|n| same_highlight(n, &e.annot)) {
                    n.text = e.annot.text.clone();
                    n.note = e.annot.note.clone();
                }
            }
        }
    }

    #[test]
    fn progress_moves_to_the_side_that_is_behind() {
        let lib = vec![
            book("L1", "Alpha", 0.2, vec![]),
            book("L2", "Beta", 0.8, vec![]),
            book("L3", "Gamma", 0.5, vec![]),
        ];
        let dev = vec![
            book("D1", "Alpha", 0.6, vec![]),
            book("D2", "Beta", 0.3, vec![]),
            book("D3", "Gamma", 0.502, vec![]),
        ];
        let p = plan(&lib, &dev);
        let t = p.totals();
        assert_eq!(
            (t.progress_to_library, t.progress_to_device, t.in_sync),
            (1, 1, 1)
        );
        assert!(
            matches!(p.books[0].progress, Progress::Forward { to_side: Side::Library, to, .. } if (to - 0.6).abs() < 1e-9)
        );
        assert!(matches!(
            p.books[1].progress,
            Progress::Forward {
                to_side: Side::Device,
                ..
            }
        ));
    }

    #[test]
    fn finished_is_never_undone_by_a_lower_number() {
        let mut dev = book("D", "Alpha", 0.4, vec![]);
        let mut lib = book("L", "Alpha", 1.0, vec![]);
        lib.status = Status::Finished;
        let p = plan(&[lib.clone()], &[dev.clone()]);
        assert!(matches!(
            p.books[0].progress,
            Progress::Forward {
                to_side: Side::Device,
                finished: true,
                ..
            }
        ));
        // finished on the device with a stale 30 % figure still counts as complete
        dev.status = Status::Finished;
        dev.progress = 0.3;
        lib.status = Status::Reading;
        lib.progress = 0.9;
        let p = plan(&[lib], &[dev]);
        assert!(
            matches!(p.books[0].progress, Progress::Forward { to_side: Side::Library, to, finished: true, .. } if to == 1.0)
        );
    }

    #[test]
    fn a_side_that_is_behind_but_read_more_recently_is_reported_not_overwritten() {
        let mut lib = book("L", "Alpha", 0.1, vec![]);
        let mut dev = book("D", "Alpha", 0.9, vec![]);
        lib.last_read = Some(1_800_000_000);
        dev.last_read = Some(1_700_000_000);
        let p = plan(&[lib], &[dev]);
        assert_eq!(p.books[0].progress, Progress::InSync);
        assert_eq!(p.books[0].behind_but_newer, Some(Side::Device));
        assert_eq!(p.books[0].last_read_to, None);
        assert_eq!(p.totals().behind_but_newer, 1);
    }

    #[test]
    fn last_read_time_flows_to_the_side_with_the_older_one() {
        let mut lib = book("L", "Alpha", 0.5, vec![]);
        let mut dev = book("D", "Alpha", 0.5, vec![]);
        dev.last_read = Some(1_800_000_000);
        let p = plan(&[lib.clone()], &[dev.clone()]);
        assert_eq!(
            p.books[0].last_read_to,
            Some((Side::Library, 1_800_000_000))
        );
        lib.last_read = Some(1_900_000_000);
        let p = plan(&[lib], &[dev]);
        assert_eq!(p.books[0].last_read_to, Some((Side::Device, 1_900_000_000)));
    }

    #[test]
    fn highlights_exist_on_both_sides_afterwards_and_nothing_is_lost() {
        let lib = vec![book(
            "L",
            "Alpha",
            0.5,
            vec![
                a("only in library text here", ""),
                a("shared highlight text", ""),
            ],
        )];
        let dev = vec![book(
            "D",
            "Alpha",
            0.5,
            vec![
                a("Shared  Highlight Text!", ""),
                a("only on device, fresh", "my note"),
            ],
        )];
        let p = plan(&lib, &dev);
        let b = &p.books[0];
        assert_eq!(
            b.notes_to_library,
            vec![a("only on device, fresh", "my note")]
        );
        assert_eq!(b.notes_to_device, vec![a("only in library text here", "")]);
        assert!(b.enrich.is_empty() && b.conflicts.is_empty());
        let t = p.totals();
        assert_eq!((t.notes_to_library, t.notes_to_device), (1, 1));
    }

    #[test]
    fn the_richer_copy_wins_and_differing_notes_are_kept_as_a_conflict() {
        let lib = vec![book(
            "L",
            "Alpha",
            0.5,
            vec![
                a("a sentence worth keeping in full", ""),
                a("another sentence worth a note", "library thought"),
            ],
        )];
        let dev = vec![book(
            "D",
            "Alpha",
            0.5,
            vec![
                a("a sentence worth keeping in full", "added on the device"),
                a("another sentence worth a note", "device thought"),
            ],
        )];
        let p = plan(&lib, &dev);
        let b = &p.books[0];
        assert_eq!(b.enrich.len(), 1);
        assert_eq!(b.enrich[0].to_side, Side::Library);
        assert_eq!(b.enrich[0].annot.note, "added on the device");
        assert_eq!(b.conflicts.len(), 1);
        assert_eq!(b.conflicts[0].library_note, "library thought");
        assert_eq!(b.conflicts[0].device_note, "device thought");
        assert!(b.notes_to_library.is_empty() && b.notes_to_device.is_empty());
    }

    #[test]
    fn a_trimmed_device_text_is_the_same_highlight_and_gets_the_fuller_text() {
        let full = "It was the best of times, it was the worst of times";
        let lib = vec![book("L", "Alpha", 0.5, vec![a(full, "")])];
        let dev = vec![book(
            "D",
            "Alpha",
            0.5,
            vec![a("It was the best of times,", "")],
        )];
        let p = plan(&lib, &dev);
        let b = &p.books[0];
        assert!(b.notes_to_library.is_empty() && b.notes_to_device.is_empty());
        assert_eq!(b.enrich.len(), 1);
        assert_eq!(b.enrich[0].to_side, Side::Device);
        assert_eq!(b.enrich[0].annot.text, full);
    }

    #[test]
    fn applying_a_plan_makes_the_next_plan_empty() {
        let mut lib = vec![
            book(
                "L1",
                "Alpha",
                0.2,
                vec![
                    a("lib only highlight text", ""),
                    a("both have this one", ""),
                ],
            ),
            book("L2", "Beta", 1.0, vec![]),
        ];
        let mut dev = vec![
            book(
                "D1",
                "Alpha",
                0.7,
                vec![
                    a("both have this one", "device note"),
                    a("dev only highlight text", "n"),
                ],
            ),
            book("D2", "Beta", 0.4, vec![]),
        ];
        let p = plan(&lib, &dev);
        assert!(!p.is_empty());
        apply(&p, &mut lib, &mut dev);
        let again = plan(&lib, &dev);
        assert!(again.is_empty(), "{again:#?}");
        // nothing was lost: every original note is still on its side
        assert!(
            lib[0]
                .notes
                .iter()
                .any(|n| n.text == "lib only highlight text")
        );
        assert!(
            dev[0]
                .notes
                .iter()
                .any(|n| n.text == "dev only highlight text")
        );
        assert!(lib[0].notes.len() == 3 && dev[0].notes.len() == 3);
        assert_eq!(lib[1].status, Status::Finished);
        assert_eq!(dev[1].status, Status::Finished);
    }

    #[test]
    fn authors_match_whatever_the_order_of_their_names() {
        let mut lib = book("1", "Building Microservices", 0.0, vec![]);
        lib.authors = vec!["Newman, Sam".into()];
        let lib = [lib];
        // what the device import gives for KOReader's "Newman, Sam;"
        assert_eq!(
            pair(&lib, "Building Microservices", &["Sam Newman".into()]),
            Pairing::One(0)
        );
        assert_eq!(
            pair(&lib, "Building Microservices", &["Newman, Sam".into()]),
            Pairing::One(0)
        );
        assert_eq!(
            pair(&lib, "Building Microservices", &["Someone Else".into()]),
            Pairing::NoBook
        );
    }

    #[test]
    fn pairing_uses_titles_then_authors_and_reports_the_rest() {
        let mut l1 = book("L1", "Dune", 0.1, vec![]);
        l1.authors = vec!["Frank Herbert".into()];
        let mut l2 = book("L2", "Dune", 0.1, vec![]);
        l2.authors = vec!["Brian Herbert".into()];
        let mut d = book("D", "01 - Dune", 0.5, vec![]);
        d.authors = vec!["brian herbert".into()];
        let p = plan(&[l1.clone(), l2.clone()], &[d.clone()]);
        assert_eq!(p.books.len(), 1);
        assert_eq!(p.books[0].library_id, "L2");
        // no authors to decide: ambiguous, left alone
        d.authors.clear();
        let p = plan(&[l1.clone(), l2], &[d.clone()]);
        assert!(p.books.is_empty());
        assert!(matches!(p.skipped[0], Skipped::Ambiguous { .. }));
        // unknown book and same title by a different author
        let p = plan(&[l1.clone()], &[book("D2", "Other", 0.3, vec![])]);
        assert!(matches!(p.skipped[0], Skipped::NotInLibrary { .. }));
        d.authors = vec!["Someone Else".into()];
        let p = plan(&[l1], &[d]);
        assert!(matches!(p.skipped[0], Skipped::NotInLibrary { .. }));
        assert_eq!(p.library_only, 1);
    }

    #[test]
    fn the_description_says_what_flows_where() {
        let lib = vec![book(
            "L",
            "Alpha",
            0.2,
            vec![a("lib only highlight text", "")],
        )];
        let dev = vec![book(
            "D",
            "Alpha",
            0.6,
            vec![a("dev only highlight text", "")],
        )];
        let s = plan(&lib, &dev).describe();
        assert!(
            s.contains("1 reading position(s) from the device to the library"),
            "{s}"
        );
        assert!(
            s.contains("1 highlight(s)/note(s) from the library to the device"),
            "{s}"
        );
        assert!(s.contains("Nothing is removed"));
        let same = plan(&lib, &lib.clone()).describe();
        assert!(same.contains("in sync"));
    }

    #[test]
    fn empty_notes_are_ignored_and_duplicates_are_not_planned_twice() {
        let lib = vec![book("L", "Alpha", 0.5, vec![])];
        let dev = vec![book(
            "D",
            "Alpha",
            0.5,
            vec![
                a("", ""),
                a("same highlight twice here", ""),
                a("same highlight twice here", ""),
            ],
        )];
        let p = plan(&lib, &dev);
        assert_eq!(p.books[0].notes_to_library.len(), 1);
    }

    #[test]
    fn a_foreign_title_is_paired_with_authors_as_tie_break_and_veto() {
        let mut a = book("1", "Dune", 0.0, vec![]);
        a.authors = vec!["Frank Herbert".into()];
        let mut b = book("2", "Dune", 0.0, vec![]);
        b.authors = vec!["Someone Else".into()];
        let lib = [a.clone(), b];
        assert_eq!(pair(&lib, "01 - Dune", &[]), Pairing::Ambiguous);
        assert_eq!(
            pair(&lib, "Dune", &["Frank Herbert".to_string()]),
            Pairing::One(0)
        );
        assert_eq!(pair(&lib, "Emma", &[]), Pairing::NoBook);
        assert_eq!(
            pair(&[a], "Dune", &["Jane Austen".to_string()]),
            Pairing::NoBook,
            "the same title by another author is another book"
        );
    }
}
