//! The single door between the app and the report: a [`Fact`] is the only thing a feature can hand to
//! the report, and a [`Redactor`] is the only thing that turns one into text.
//!
//! A `Fact` cannot be made from a `String`. A feature chooses what kind of fact it is telling:
//!
//! * [`Fact::word`], [`Fact::count`], [`Fact::flag`], [`Fact::size`] ... : a literal, a number, a class.
//!   Shown as is, because none of them can hold text a person wrote.
//! * [`Fact::choice`]: a value from the outside (a file system type, a renderer name) that is shown
//!   only when it is one of the words the caller lists, else `other`.
//! * [`Fact::text`]: free text (an error message). Cleaned of addresses, paths and file names, then by
//!   the scrubber (home folder, library folder, titles, servers, accounts, secrets, addresses).
//! * [`Fact::address`]: a server address, shown by its shape ([`crate::diag_addr::shape`]); the host
//!   only when the user allowed it in the dialog.
//! * [`Fact::title`]: the title of a book: shown only when the user allowed it in the dialog.
//!
//! A [`Section`] is a titled list of `(label, Fact)`. Rendering is [`Redactor::section`]; the report
//! builder has no other way to put a new fact into the report. Adding a fact: see
//! `docs/diagnostic-report-notes.md`, "How to add a fact safely".
//!
//! No GTK, no I/O.

use crate::debug_log::Scrubber;
use crate::diag_addr;

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// Already safe by construction.
    Safe(String),
    Text(String),
    Address(String),
    Title(String),
}

/// One fact of the report. See the module text.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact(Kind);

impl Fact {
    /// A literal of the source.
    pub fn word(w: &'static str) -> Fact {
        Fact(Kind::Safe(w.to_string()))
    }
    pub fn count(n: usize) -> Fact {
        Fact(Kind::Safe(n.to_string()))
    }
    pub fn num(n: u64) -> Fact {
        Fact(Kind::Safe(n.to_string()))
    }
    pub fn flag(on: bool) -> Fact {
        Fact::word(if on { "yes" } else { "no" })
    }
    pub fn onoff(on: bool) -> Fact {
        Fact::word(if on { "on" } else { "off" })
    }
    /// A size as a class (`lt10mb`).
    pub fn size(bytes: u64) -> Fact {
        Fact::word(crate::trace::size_class(bytes))
    }
    /// A count as a class (`100-999`), for numbers that would identify a person when exact.
    pub fn count_class(n: u64) -> Fact {
        Fact::word(crate::trace::count_class(n))
    }
    #[allow(dead_code)] // part of the vocabulary; features use it as they gain durations
    pub fn ms(ms: u64) -> Fact {
        Fact(Kind::Safe(format!("{ms} ms")))
    }
    pub fn ratio(v: f64) -> Fact {
        Fact(Kind::Safe(format!("{:.3}", v.clamp(0.0, 1.0))))
    }
    /// A hash of bytes; anything else is not shown.
    #[allow(dead_code)] // part of the vocabulary (used by the tests of this module)
    pub fn hash(h: &str) -> Fact {
        Fact(Kind::Safe(
            crate::trace::hex_id(h).unwrap_or_else(|| "(not a hash)".into()),
        ))
    }
    /// A version number (`4.24.1`, `0.3.0-beta.2`); anything else is not shown.
    pub fn version(v: &str) -> Fact {
        let ok = !v.is_empty()
            && v.len() <= 40
            && v.starts_with(|c: char| c.is_ascii_digit())
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '~' | '_'));
        Fact(Kind::Safe(if ok {
            v.to_string()
        } else {
            "(unknown)".into()
        }))
    }
    /// A value from outside, shown only when (case-insensitively) one of `allowed`.
    pub fn choice(value: &str, allowed: &[&str]) -> Fact {
        let v = value.trim();
        Fact(Kind::Safe(
            allowed
                .iter()
                .find(|a| a.eq_ignore_ascii_case(v))
                .map_or_else(|| "other".to_string(), |a| (*a).to_string()),
        ))
    }
    /// Several choices at once (`sockets: wayland, x11`): each one must be allowed.
    pub fn choices<S: AsRef<str>>(values: &[S], allowed: &[&str]) -> Fact {
        let v: Vec<String> = values
            .iter()
            .map(|s| match Fact::choice(s.as_ref(), allowed).0 {
                Kind::Safe(s) => s,
                _ => "other".into(),
            })
            .collect();
        Fact(Kind::Safe(if v.is_empty() {
            "none".into()
        } else {
            v.join(", ")
        }))
    }
    /// Several values, each accepted by `ok` (a check of the exact shape the caller produces).
    pub fn choices_by<S: AsRef<str>>(values: &[S], ok: &dyn Fn(&str) -> bool) -> Fact {
        let v: Vec<&str> = values
            .iter()
            .map(|s| if ok(s.as_ref()) { s.as_ref() } else { "other" })
            .collect();
        Fact(Kind::Safe(if v.is_empty() {
            "none".into()
        } else {
            v.join(", ")
        }))
    }
    /// A time zone offset from UTC, in minutes: `UTC+02:00`.
    pub fn utc_offset(minutes: i32) -> Fact {
        let sign = if minutes < 0 { '-' } else { '+' };
        let m = minutes.abs();
        Fact(Kind::Safe(format!("UTC{sign}{:02}:{:02}", m / 60, m % 60)))
    }
    /// A language code (`en`, `pt-BR`): two or three letters, then an optional region.
    pub fn lang(code: &str) -> Fact {
        let c = code.trim();
        let ok = {
            let mut parts = c.split(['-', '_']);
            let first = parts.next().unwrap_or("");
            (2..=3).contains(&first.len())
                && first.chars().all(|ch| ch.is_ascii_alphabetic())
                && parts.clone().count() <= 2
                && parts.all(|p| {
                    (2..=4).contains(&p.len()) && p.chars().all(|ch| ch.is_ascii_alphanumeric())
                })
        };
        Fact(Kind::Safe(if ok {
            c.to_string()
        } else {
            "unknown".into()
        }))
    }
    /// HTTP statuses with how often each was seen: `401×3, 404×1`.
    pub fn statuses(v: &[(u16, usize)]) -> Fact {
        Fact(Kind::Safe(
            v.iter()
                .take(8)
                .map(|(s, n)| format!("{s}×{n}"))
                .collect::<Vec<_>>()
                .join(", "),
        ))
    }
    /// Free text: cleaned when shown.
    pub fn text(s: &str) -> Fact {
        Fact(Kind::Text(s.to_string()))
    }
    pub fn address(raw: &str) -> Fact {
        Fact(Kind::Address(raw.to_string()))
    }
    pub fn title(t: &str) -> Fact {
        Fact(Kind::Title(t.to_string()))
    }
}

/// A titled list of facts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Section {
    pub title: String,
    pub rows: Vec<(String, Fact)>,
}

impl Section {
    pub fn new(title: &str) -> Section {
        Section {
            title: title.to_string(),
            rows: Vec::new(),
        }
    }
    pub fn row(&mut self, label: &str, fact: Fact) -> &mut Section {
        self.rows.push((label.to_string(), fact));
        self
    }
}

/// Turns facts into report text under the dialog's choices.
pub struct Redactor<'a> {
    pub scrub: &'a Scrubber,
    /// The user allowed the host of server addresses.
    pub show_host: bool,
    /// The user allowed the titles and authors of the books involved.
    pub show_titles: bool,
}

/// Addresses by shape, then the scrubber (it still sees the library folder and the titles whole), then
/// the paths and file names that are left.
pub fn clean_full(scrub: &Scrubber, s: &str) -> String {
    diag_addr::clean_paths(&scrub.apply(&diag_addr::clean_urls(s)))
}

/// Longest title or author line shown.
const TITLE_BYTES: usize = 120;

impl Redactor<'_> {
    /// Free text, cleaned: addresses, paths and file names first (while they are whole), then the scrubber.
    pub fn text(&self, s: &str) -> String {
        clean_full(self.scrub, s)
    }

    /// One fact as one line of text (no newline, no Markdown that would break the list).
    pub fn fact(&self, f: &Fact) -> String {
        let one = |s: &str| -> String {
            s.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect::<String>()
                .trim()
                .to_string()
        };
        match &f.0 {
            Kind::Safe(s) => one(s),
            Kind::Text(s) => one(&self.text(s.lines().next().unwrap_or(""))),
            Kind::Address(a) => diag_addr::shape(&one(a), self.show_host),
            Kind::Title(t) => {
                if self.show_titles {
                    let mut t = one(t).replace('`', "'");
                    if t.len() > TITLE_BYTES {
                        let mut c = TITLE_BYTES;
                        while !t.is_char_boundary(c) {
                            c -= 1;
                        }
                        t.truncate(c);
                        t.push('…');
                    }
                    t
                } else {
                    "(not included)".into()
                }
            }
        }
    }

    /// `### Title` and the list; an empty section is not shown.
    pub fn section(&self, s: &Section) -> String {
        if s.rows.is_empty() {
            return String::new();
        }
        let label = |l: &str| -> String {
            l.chars()
                .filter(|c| c.is_ascii_alphanumeric() || " /().,-_+:%".contains(*c))
                .collect()
        };
        let mut out = format!("### {}\n\n", label(&s.title));
        for (k, f) in &s.rows {
            out.push_str(&format!("- {}: {}\n", label(k), self.fact(f)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug_log::Scrub;

    fn scrub() -> Scrubber {
        Scrubber::new(Scrub {
            home: "/home/alice".into(),
            user: "alice".into(),
            library: "/home/alice/Books".into(),
            titles: vec!["The Secret Garden".into()],
            secrets: vec!["hunter2-pass".into()],
            hosts: vec!["sync.example.org".into()],
            accounts: vec!["alice_reads".into()],
        })
    }

    fn r(s: &Scrubber, host: bool, titles: bool) -> Redactor<'_> {
        Redactor {
            scrub: s,
            show_host: host,
            show_titles: titles,
        }
    }

    #[test]
    fn each_kind_of_fact_is_shown_the_way_it_promises() {
        let s = scrub();
        let r = r(&s, false, false);
        assert_eq!(r.fact(&Fact::flag(true)), "yes");
        assert_eq!(r.fact(&Fact::size(3 << 20)), "lt10mb");
        assert_eq!(r.fact(&Fact::ms(12)), "12 ms");
        assert_eq!(
            r.fact(&Fact::hash("0123456789abcdef0123456789abcdef")),
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(r.fact(&Fact::hash("The Secret Garden")), "(not a hash)");
        assert_eq!(r.fact(&Fact::version("4.24.1")), "4.24.1");
        assert_eq!(r.fact(&Fact::version("alice")), "(unknown)");
        assert_eq!(r.fact(&Fact::choice("EXT4", &["ext4", "btrfs"])), "ext4");
        assert_eq!(r.fact(&Fact::choice("alice-nas", &["ext4"])), "other");
        assert_eq!(
            r.fact(&Fact::choices(
                &["wayland", "x11", "alice"],
                &["wayland", "x11"]
            )),
            "wayland, x11, other"
        );
        assert_eq!(r.fact(&Fact::title("The Secret Garden")), "(not included)");
        assert_eq!(
            r.fact(&Fact::address("http://192.168.1.5:30260/api/koreader")),
            "http://<ip:private>:30260/api/koreader"
        );
    }

    #[test]
    fn free_text_is_cleaned_by_addresses_then_scrubber() {
        let s = scrub();
        let r = r(&s, false, false);
        let t = r.fact(&Fact::text(
            "cannot open /home/alice/Books/The Secret Garden.epub for alice_reads at http://sync.example.org/api\nsecond line",
        ));
        for bad in ["alice", "Secret", "sync.example.org", "Garden"] {
            assert!(!t.contains(bad), "{bad} in {t}");
        }
        assert!(!t.contains('\n') && !t.contains("second"));
    }

    #[test]
    fn titles_and_hosts_appear_only_when_allowed() {
        let s = scrub();
        let on = r(&s, true, true);
        assert_eq!(
            on.fact(&Fact::title("The Secret Garden")),
            "The Secret Garden"
        );
        assert_eq!(
            on.fact(&Fact::address(
                "https://bob:pw@books.example.org:8443/x?t=1"
            )),
            "https://<credentials>@books.example.org:8443/<segment>?<query>"
        );
        assert!(on.fact(&Fact::title(&"x`y".repeat(200))).len() <= TITLE_BYTES + 4);
        assert!(!on.fact(&Fact::title("a`b")).contains('`'));
    }

    #[test]
    fn a_section_is_a_list_and_an_empty_one_is_nothing() {
        let s = scrub();
        let r = r(&s, false, false);
        let mut sec = Section::new("Read aloud");
        assert_eq!(r.section(&sec), "");
        sec.row("Engine", Fact::word("piper"))
            .row("Voices", Fact::count(2));
        assert_eq!(
            r.section(&sec),
            "### Read aloud\n\n- Engine: piper\n- Voices: 2\n"
        );
    }

    #[test]
    fn labels_cannot_inject_markup_or_lines() {
        let s = scrub();
        let r = r(&s, false, false);
        let mut sec = Section::new("Bad\n### Title");
        sec.row("a\n- b: `x`", Fact::word("v"));
        let t = r.section(&sec);
        assert_eq!(t.lines().count(), 3, "{t}");
        assert!(!t.contains('`'));
    }
}
