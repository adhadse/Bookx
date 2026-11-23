//! Bookx as a KOSync client: the settings, how a place in a book is told to and read from a
//! KOReader-style server, and the rules for when to ask and when to push. The protocol itself is
//! `bookx_serve::kosync_client`; the widgets are in `ui::kosync`. No GTK in here.
//!
//! **Rules**
//!
//! * Opening a book pulls the server's position. If it comes from another device and is further
//!   along than here by more than 1 %, the user is asked "Continue from <device> at 63 %?". Nothing
//!   ever moves the reader silently, and a position behind the local one is never offered.
//! * Page turns push the new position after a pause (the debounce), closing the book pushes at once.
//! * A push never takes the server backwards by surprise: until the user has answered the question
//!   (or the server was seen to have nothing ahead), a push is held back when the server's position
//!   is further along and comes from another device ([`push_allowed`]).
//!
//! **Position mapping (limits)** — KOReader tells a place as an *xpointer* into its own rendering
//! of the book plus a *percentage*. Bookx cannot place an xpointer in its own layout, so the
//! percentage decides: it is turned into a chapter and a place in the chapter with the same
//! chapter weights Bookx uses for its own progress bar. Expect the result to be off by a page or
//! two, more in books with large images or many short chapters. What Bookx writes as its xpointer
//! is the start of the chapter (`/body/DocFragment[N]/body`, which KOReader understands), so
//! KOReader opens at the start of that chapter rather than at the exact line; the percentage is
//! exact for Bookx's own reader.

use crate::i18n::trf;
use bookx_serve::kosync_client::{Client, DocMethod, Remote, document_id};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The *Sync reading position* settings. The password is kept in the system keyring, like the
/// share and mail passwords.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KoSyncPrefs {
    pub enabled: bool,
    pub server: String,
    pub user: String,
    pub password: String,
    /// `true`: name books by the MD5 of their file name; `false`: by the partial MD5 of the file.
    pub by_filename: bool,
    /// Shown on the other devices ("Continue from <this name>?").
    pub device_name: String,
    /// Tells this installation apart from the others; made on first use.
    pub device_id: String,
    /// Register or Log in worked for this server, user name and password; cleared when one of
    /// them is edited. Lets the page say “Logged in as …” after a restart.
    pub logged_in: bool,
}

impl KoSyncPrefs {
    pub fn method(&self) -> DocMethod {
        if self.by_filename {
            DocMethod::Filename
        } else {
            DocMethod::Binary
        }
    }

    /// Whether syncing can happen at all: switched on and the server, user and password are there.
    pub fn ready(&self) -> bool {
        self.enabled
            && !self.server.trim().is_empty()
            && !self.user.trim().is_empty()
            && !self.password.is_empty()
    }

    pub fn client(&self) -> anyhow::Result<Client> {
        Client::new(&self.server, &self.user, &self.password)
    }

    /// The name other devices show for this one.
    pub fn display_name(&self) -> String {
        let n = self.device_name.trim();
        if n.is_empty() {
            "Bookx".to_string()
        } else {
            n.to_string()
        }
    }

    /// Makes the device id on first use; returns whether it was made (the config needs saving).
    pub fn ensure_device_id(&mut self) -> bool {
        if self.device_id.is_empty() {
            self.device_id = uuid::Uuid::new_v4().simple().to_string().to_uppercase();
            true
        } else {
            false
        }
    }
}

/// The flight-recorder entry of one KOSync call about a book: its format, size class, library id and
/// the document matching method (never its name or path). The caller finishes it with `ok` or `fail`.
pub fn trace_op(
    name: &'static str,
    book_id: u64,
    path: &std::path::Path,
    prefs: &KoSyncPrefs,
) -> crate::trace::Op {
    let mut t = crate::trace::op(crate::trace::Area::Sync, name);
    t.book(book_id).file(path).word(
        "method",
        if prefs.by_filename {
            "filename"
        } else {
            "partial-md5"
        },
    );
    t
}

// -- position mapping ---------------------------------------------------------------------------

/// The share of the book before the place `frac` of chapter `spine`, by the chapter `weights`
/// (what the reader computes for its progress; here to check [`position_of`] is its inverse).
#[cfg(test)]
pub fn percentage_of(weights: &[f64], spine: usize, frac: f64) -> f64 {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let before: f64 = weights[..spine.min(weights.len())].iter().sum();
    let cur = weights.get(spine).copied().unwrap_or(0.0);
    ((before + cur * frac.clamp(0.0, 1.0)) / total).clamp(0.0, 1.0)
}

/// The chapter and the place in it for a share `pct` of the book (the inverse of [`percentage_of`]).
pub fn position_of(weights: &[f64], pct: f64) -> (usize, f64) {
    let total: f64 = weights.iter().sum();
    if weights.is_empty() || total <= 0.0 {
        return (0, 0.0);
    }
    let target = pct.clamp(0.0, 1.0) * total;
    let mut before = 0.0;
    for (i, w) in weights.iter().enumerate() {
        if target < before + w || i + 1 == weights.len() {
            let frac = if *w > 0.0 { (target - before) / w } else { 0.0 };
            return (i, frac.clamp(0.0, 1.0));
        }
        before += w;
    }
    (0, 0.0)
}

/// What Bookx writes as KOReader's `progress`: the start of the chapter.
pub fn progress_string(spine: usize) -> String {
    format!("/body/DocFragment[{}]/body", spine + 1)
}

// -- deciding -----------------------------------------------------------------------------------

/// A gap smaller than this is not worth a question (1 % of the book).
pub const ASK_GAP: f64 = 0.01;
/// A change smaller than this is not pushed.
pub const PUSH_STEP: f64 = 0.001;

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Nothing,
    /// Ask "Continue from <device> at <percentage>?".
    Offer {
        percentage: f64,
        device: String,
        when: i64,
    },
}

/// What to do with the server's position when a book is opened at `local`.
pub fn decide(local: f64, remote: &Remote, my_device_id: &str) -> Decision {
    if !my_device_id.is_empty() && remote.device_id == my_device_id {
        return Decision::Nothing;
    }
    if remote.percentage > local + ASK_GAP {
        Decision::Offer {
            percentage: remote.percentage,
            device: if remote.device.trim().is_empty() {
                "another device".to_string()
            } else {
                remote.device.clone()
            },
            when: remote.timestamp,
        }
    } else {
        Decision::Nothing
    }
}

/// The question the dialog asks.
pub fn prompt_title(device: &str, percentage: f64) -> String {
    trf(
        "Continue from {} at {}%?",
        &[&device, &format!("{:.0}", percentage * 100.0)],
    )
}

pub fn prompt_body(local: f64, when: i64) -> String {
    let date = chrono::DateTime::from_timestamp(when, 0)
        .filter(|_| when > 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%-d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    trf(
        "You are at {}% here{}. “Stay Here” keeps your place; nothing is changed unless you choose to continue.",
        &[
            &format!("{:.0}", local * 100.0),
            &(if date.is_empty() {
                String::new()
            } else {
                format!("; the other position was saved on {date}")
            }),
        ],
    )
}

/// The document ids a book may be known by on the server, best first. With *partial MD5* these
/// are the ids of the file as it was imported (what the server's copy and the e-reader have; Bookx
/// rewrites an EPUB and turns a PDF into an EPUB), then of the other format files (the original
/// PDF) and last of the library file as it is now. With *file name* every file's name.
/// The read of a file that cannot be opened is skipped; the list is never empty unless no file
/// can be read.
pub fn candidate_ids(source_ids: &[String], files: &[PathBuf], method: DocMethod) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |id: String| {
        if !out.contains(&id) {
            out.push(id);
        }
    };
    if method == DocMethod::Binary {
        for id in source_ids {
            add(id.clone());
        }
    }
    // the primary file is first in `files`; the others come before it so the original wins
    let (first, rest) = files
        .split_first()
        .map_or((None, &[][..]), |(f, r)| (Some(f), r));
    for f in rest.iter().chain(first) {
        if let Ok(id) = document_id(f, method) {
            add(id);
        }
    }
    out
}

/// `ids` with the id the server already has a position under (found when the book was opened)
/// first, so a book synced under one id keeps using it.
pub fn prefer(ids: Vec<String>, known: Option<&str>) -> Vec<String> {
    let Some(k) = known else {
        return ids;
    };
    let mut out = vec![k.to_string()];
    out.extend(ids.into_iter().filter(|i| i != k));
    out
}

/// What the user is told when the server does not know a book (a push was answered 404).
pub fn unknown_book_text(id: &str) -> String {
    trf(
        "The sync server does not know this book (id {}). Grimmory and some other servers recognise a book by the content of its file, so the file in Bookx must be identical to the one on the server. Add the book from the server again, or check “Match books by” in Preferences → Sync → Advanced.",
        &[&id],
    )
}

/// Whether a position at `pct` is worth pushing, given the last one the server is known to have.
pub fn worth_pushing(last: f64, pct: f64) -> bool {
    (pct - last).abs() > PUSH_STEP
}

/// Whether a push of `local` may go out now. It may when the user already answered the question
/// for this book, or when the server has nothing, or nothing further along from another device.
pub fn push_allowed(
    remote: Option<&Remote>,
    local: f64,
    my_device_id: &str,
    answered: bool,
) -> bool {
    if answered {
        return true;
    }
    match remote {
        None => true,
        Some(r) => matches!(decide(local, r, my_device_id), Decision::Nothing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(pct: f64, device: &str, id: &str) -> Remote {
        Remote {
            document: "d".into(),
            progress: "/body/DocFragment[3]/body/p[2]/text().10".into(),
            percentage: pct,
            device: device.into(),
            device_id: id.into(),
            timestamp: 1_700_000_000,
        }
    }

    fn tmp_file(tag: &str, data: &[u8]) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-kosync-cand-{tag}-{}-{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("t")
                .replace("::", "_")
        ));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join(format!("{tag}.bin"));
        std::fs::write(&f, data).unwrap();
        f
    }

    #[test]
    fn a_book_bookx_rewrote_is_known_by_the_id_of_the_file_it_came_from() {
        // regression for issue 49: the library copy hashes differently from the server's file
        let library_copy = tmp_file("copy", b"rewritten epub bytes");
        let original = tmp_file("orig", b"original epub bytes");
        let orig_id = bookx_serve::kosync_client::partial_md5(&original).unwrap();
        let copy_id = bookx_serve::kosync_client::partial_md5(&library_copy).unwrap();
        assert_ne!(orig_id, copy_id);
        let ids = candidate_ids(
            std::slice::from_ref(&orig_id),
            std::slice::from_ref(&library_copy),
            DocMethod::Binary,
        );
        assert_eq!(ids, vec![orig_id.clone(), copy_id.clone()]);
        // a book imported before the id was kept: only the library file
        assert_eq!(
            candidate_ids(&[], std::slice::from_ref(&library_copy), DocMethod::Binary),
            vec![copy_id.clone()]
        );
        // the source id is not repeated when the library file is the original
        assert_eq!(
            candidate_ids(
                std::slice::from_ref(&orig_id),
                std::slice::from_ref(&original),
                DocMethod::Binary
            ),
            vec![orig_id]
        );
    }

    #[test]
    fn the_id_the_server_already_has_is_tried_first() {
        let ids = vec!["src".to_string(), "copy".to_string()];
        assert_eq!(prefer(ids.clone(), None), ids);
        assert_eq!(prefer(ids.clone(), Some("copy")), vec!["copy", "src"]);
        assert_eq!(prefer(ids, Some("old")), vec!["old", "src", "copy"]);
    }

    #[test]
    fn the_original_pdf_comes_before_the_epub_made_from_it() {
        let epub = tmp_file("made", b"epub made from a pdf");
        let pdf = tmp_file("pdf", b"%PDF-1.4 original");
        let pdf_id = bookx_serve::kosync_client::partial_md5(&pdf).unwrap();
        let epub_id = bookx_serve::kosync_client::partial_md5(&epub).unwrap();
        assert_eq!(
            candidate_ids(&[], &[epub, pdf], DocMethod::Binary),
            vec![pdf_id, epub_id]
        );
    }

    #[test]
    fn by_file_name_the_source_ids_do_not_count() {
        let f = tmp_file("named", b"x");
        let ids = candidate_ids(
            &["abc".into()],
            std::slice::from_ref(&f),
            DocMethod::Filename,
        );
        assert_eq!(ids.len(), 1);
        assert_ne!(ids[0], "abc");
    }

    #[test]
    fn the_unknown_book_message_explains_what_to_do() {
        let t = unknown_book_text("abc123");
        assert!(t.contains("abc123"));
        assert!(t.contains("content of its file"));
        assert!(t.contains("Match books by"));
    }

    #[test]
    fn percentage_and_position_are_inverse_by_the_chapter_weights() {
        let w = [10.0, 30.0, 60.0];
        assert_eq!(percentage_of(&w, 0, 0.0), 0.0);
        assert_eq!(percentage_of(&w, 1, 0.5), 0.25);
        assert_eq!(percentage_of(&w, 2, 1.0), 1.0);
        for (s, f) in [(0, 0.3), (1, 0.0), (1, 0.9), (2, 0.5)] {
            let (s2, f2) = position_of(&w, percentage_of(&w, s, f));
            assert_eq!(s2, s);
            assert!((f2 - f).abs() < 1e-9, "{s} {f} -> {f2}");
        }
        assert_eq!(position_of(&w, 0.63), (2, (0.63 * 100.0 - 40.0) / 60.0));
        assert_eq!(position_of(&w, 1.0), (2, 1.0));
        assert_eq!(position_of(&w, 2.0), (2, 1.0), "out of range is clamped");
        assert_eq!(position_of(&[], 0.5), (0, 0.0));
        assert_eq!(percentage_of(&[], 0, 0.5), 0.0);
        // an empty chapter in the middle is skipped, not divided by
        assert_eq!(position_of(&[50.0, 0.0, 50.0], 0.5), (2, 0.0));
    }

    #[test]
    fn the_xpointer_bookx_writes_names_the_chapter_in_koreaders_terms() {
        assert_eq!(progress_string(0), "/body/DocFragment[1]/body");
        assert_eq!(progress_string(2), "/body/DocFragment[3]/body");
    }

    #[test]
    fn a_position_from_another_device_that_is_ahead_is_offered_never_applied() {
        let me = "ME";
        assert_eq!(
            decide(0.40, &remote(0.63, "Kobo Libra", "K1"), me),
            Decision::Offer {
                percentage: 0.63,
                device: "Kobo Libra".into(),
                when: 1_700_000_000
            }
        );
        // behind or about equal: nothing, we never offer to move backwards
        assert_eq!(
            decide(0.70, &remote(0.63, "Kobo", "K1"), me),
            Decision::Nothing
        );
        assert_eq!(
            decide(0.625, &remote(0.63, "Kobo", "K1"), me),
            Decision::Nothing
        );
        // our own push
        assert_eq!(
            decide(0.10, &remote(0.63, "Bookx", me), me),
            Decision::Nothing
        );
        // an unnamed device
        match decide(0.0, &remote(0.5, " ", "K1"), me) {
            Decision::Offer { device, .. } => assert_eq!(device, "another device"),
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn a_push_does_not_take_the_server_backwards_before_the_user_answered() {
        let me = "ME";
        let ahead = remote(0.63, "Kobo", "K1");
        assert!(!push_allowed(Some(&ahead), 0.40, me, false));
        assert!(
            push_allowed(Some(&ahead), 0.40, me, true),
            "after the question it is the user's choice"
        );
        assert!(
            push_allowed(Some(&ahead), 0.70, me, false),
            "we are the furthest"
        );
        assert!(push_allowed(None, 0.10, me, false), "nothing on the server");
        assert!(
            push_allowed(Some(&remote(0.9, "Bookx", me)), 0.10, me, false),
            "our own earlier push"
        );
    }

    #[test]
    fn tiny_changes_are_not_pushed() {
        assert!(!worth_pushing(0.5, 0.5004));
        assert!(worth_pushing(0.5, 0.502));
        assert!(worth_pushing(0.5, 0.49), "backwards is a change too");
    }

    #[test]
    fn the_prompt_says_where_from_and_how_far() {
        assert_eq!(
            prompt_title("Kobo Libra", 0.63),
            "Continue from Kobo Libra at 63%?"
        );
        let b = prompt_body(0.4, 1_700_000_000);
        assert!(b.contains("You are at 40% here"), "{b}");
        assert!(b.contains("Nov 2023"), "{b}");
        assert!(prompt_body(0.4, 0).contains("here."));
    }

    #[test]
    fn settings_need_everything_and_make_a_device_id_once() {
        let mut p = KoSyncPrefs::default();
        assert!(!p.ready());
        p.enabled = true;
        p.server = "https://sync.koreader.rocks".into();
        p.user = "me".into();
        assert!(!p.ready(), "no password");
        p.password = "pw".into();
        assert!(p.ready());
        assert!(p.client().is_ok());
        assert_eq!(p.display_name(), "Bookx");
        assert!(p.ensure_device_id());
        let id = p.device_id.clone();
        assert_eq!(id.len(), 32);
        assert!(!p.ensure_device_id());
        assert_eq!(p.device_id, id);
        assert_eq!(p.method(), DocMethod::Binary);
        p.by_filename = true;
        assert_eq!(p.method(), DocMethod::Filename);
        let q: KoSyncPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(q, KoSyncPrefs::default());
    }
}
