//! The text of *Sync activity* (Sync window and Preferences → Sync): which e-readers talked to the
//! Bookx sync server and what they sent. No GTK in here and no clock of its own: every function takes
//! `now` (Unix seconds), so the tests can say what time it is. The facts come from
//! `bookx_serve::activity` (devices and events written by the server) and the ledger of
//! `annotation_sync` (what Bookx could not match or place). Written for people: no technical ids except
//! the short hash of a book that is not in the library.

use std::collections::{BTreeSet, HashMap};

use bookx_serve::activity::{Device, Event, Presence, UNKNOWN_DEVICE, What};

use crate::i18n::{ntr, tr, trf};

/// "just now", "5 minutes ago", "3 hours ago", "2 days ago".
pub fn relative(now: i64, then: i64) -> String {
    let ago = (now - then).max(0);
    if ago < 60 {
        tr("just now")
    } else if ago < 3600 {
        ntr("{} minute ago", "{} minutes ago", (ago / 60) as u64)
    } else if ago < 86_400 {
        ntr("{} hour ago", "{} hours ago", (ago / 3600) as u64)
    } else {
        ntr("{} day ago", "{} days ago", (ago / 86_400) as u64)
    }
}

/// The name of a device for people (an unnamed one is "Unknown device (KOReader)").
pub fn device_name(d: &str) -> String {
    if d.trim().is_empty() || d == UNKNOWN_DEVICE {
        tr("Unknown device (KOReader)")
    } else {
        d.to_string()
    }
}

/// One row of *Connected devices*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// "Connected now", "Last seen 5 minutes ago", "Never connected".
    pub status: String,
    pub connected: bool,
}

pub fn device_row(d: &Device, now: i64) -> DeviceRow {
    let mut parts = Vec::new();
    parts.push(if d.client.is_empty() {
        tr("KOReader")
    } else {
        d.client.clone()
    });
    parts.push(if d.plugin.is_empty() {
        tr("no Bookx plugin (reading position only)")
    } else {
        trf("plugin {}", &[&d.plugin])
    });
    parts.push(match d.last_sync {
        Some(t) => trf("last sync {}", &[&relative(now, t)]),
        None => tr("nothing synced yet"),
    });
    let (status, connected) = if d.last_seen <= 0 {
        (tr("Never connected"), false)
    } else {
        match d.presence(now) {
            Presence::Connected => (tr("Connected now"), true),
            Presence::Seen { seconds } => {
                (trf("Last seen {}", &[&relative(now, now - seconds)]), false)
            }
        }
    };
    DeviceRow {
        id: d.id.clone(),
        title: device_name(&d.name),
        subtitle: parts.join(" · "),
        status,
        connected,
    }
}

/// The first eight characters of a book hash, for the rows that cannot name the book.
pub fn short_hash(doc: &str) -> String {
    doc.chars().take(8).collect()
}

/// Book titles by document hash: what the library knows (matched books) first, then what the
/// device called the book.
pub fn title_for(doc: &str, ev_title: &str, known: &HashMap<String, String>) -> String {
    if let Some(t) = known.get(doc).filter(|t| !t.trim().is_empty()) {
        return t.clone();
    }
    if !ev_title.trim().is_empty() {
        return ev_title.trim().to_string();
    }
    if doc.is_empty() {
        tr("Unknown book")
    } else {
        trf("Unknown book ({})", &[&short_hash(doc)])
    }
}

fn pct(x: f64) -> String {
    format!("{}%", (x * 100.0).round() as i64)
}

/// What an event says, in one line.
pub fn what_text(w: &What, doc: &str) -> String {
    match w {
        What::Progress { from: Some(a), to } => trf("Progress {} → {}", &[&pct(*a), &pct(*to)]),
        What::Progress { from: None, to } => trf("Progress {}", &[&pct(*to)]),
        What::Received {
            highlights,
            bookmarks,
            notes,
            removed,
        } => {
            let mut p = Vec::new();
            if *highlights > 0 {
                p.push(ntr(
                    "{} highlight added or updated",
                    "{} highlights added or updated",
                    *highlights,
                ));
            }
            if *notes > 0 {
                p.push(ntr("{} with a note", "{} with notes", *notes));
            }
            if *bookmarks > 0 {
                p.push(ntr("{} bookmark", "{} bookmarks", *bookmarks));
            }
            if *removed > 0 {
                p.push(ntr("{} removed", "{} removed", *removed));
            }
            if p.is_empty() {
                tr("Nothing new")
            } else {
                p.join(", ")
            }
        }
        What::Nothing => tr("Nothing new"),
        What::Rejected { reason, status } => {
            if reason.is_empty() {
                trf("Server rejected it (status {})", &[status])
            } else {
                trf("Server rejected it: {}", &[reason])
            }
        }
        What::NotInLibrary { count } => trf(
            "Book not found in your library (hash {}): {}",
            &[
                &short_hash(doc),
                &ntr(
                    "{} highlight not imported",
                    "{} highlights not imported",
                    *count,
                ),
            ],
        ),
        What::Unplaced { count } => ntr(
            "{} highlight could not be placed in the book",
            "{} highlights could not be placed in the book",
            *count,
        ),
    }
}

/// One row of *Recent activity*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRow {
    pub title: String,
    pub subtitle: String,
    /// Something went wrong or was left out (shown with a warning style).
    pub problem: bool,
    pub time: i64,
}

pub fn activity_row(e: &Event, now: i64, known: &HashMap<String, String>) -> ActivityRow {
    let book = if e.document.is_empty() && e.title.is_empty() {
        match e.what {
            What::Rejected { .. } => tr("Request refused"),
            _ => tr("Unknown book"),
        }
    } else {
        title_for(&e.document, &e.title, known)
    };
    ActivityRow {
        title: book,
        subtitle: format!(
            "{} · {} · {}",
            what_text(&e.what, &e.document),
            device_name(&e.device),
            relative(now, e.time)
        ),
        problem: matches!(
            e.what,
            What::Rejected { .. } | What::NotInLibrary { .. } | What::Unplaced { .. }
        ),
        time: e.time,
    }
}

/// The line over the list: "Today: 12 highlights, 3 bookmarks, progress for 2 books".
/// `since` is the start of today (local midnight) as Unix seconds.
pub fn today_line(events: &[Event], since: i64) -> String {
    let (mut h, mut b, mut n) = (0u32, 0u32, 0u32);
    let mut progress = BTreeSet::new();
    for e in events.iter().filter(|e| e.time >= since) {
        match &e.what {
            What::Received {
                highlights,
                bookmarks,
                notes,
                ..
            } => {
                h += highlights;
                b += bookmarks;
                n += notes;
            }
            What::Progress { .. } => {
                progress.insert(e.document.clone());
            }
            _ => {}
        }
    }
    let mut p = Vec::new();
    if h > 0 {
        p.push(ntr("{} highlight", "{} highlights", h));
    }
    if b > 0 {
        p.push(ntr("{} bookmark", "{} bookmarks", b));
    }
    if n > 0 {
        p.push(ntr("{} note", "{} notes", n));
    }
    if !progress.is_empty() {
        p.push(trf(
            "progress for {}",
            &[&ntr("{} book", "{} books", progress.len())],
        ));
    }
    if p.is_empty() {
        tr("Nothing synced today")
    } else {
        trf("Today: {}", &[&p.join(", ")])
    }
}

/// The text of the empty list.
pub fn empty_text(address: Option<&str>) -> String {
    let mut s = tr(
        "Nothing has synced yet. On your e-reader open a book, then More tools, Bookx Sync, Sync now.",
    );
    match address {
        Some(a) => {
            s.push(' ');
            s.push_str(&trf(
                "Server address for the e-reader: {}. Use Test connection in the plugin to check it.",
                &[&a],
            ));
        }
        None => {
            s.push(' ');
            s.push_str(&tr(
                "The Bookx sync server is not running: turn it on first.",
            ));
        }
    }
    s
}

/// A highlight, bookmark or note that arrived and what became of it, for the toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    pub book_id: u64,
    pub title: String,
    pub device: String,
    pub highlights: usize,
    pub bookmarks: usize,
}

/// "Synced 3 highlights from Kobo Libra for Building Microservices". `None` when nothing arrived.
pub fn toast_text(a: &[Arrival]) -> Option<String> {
    let h: usize = a.iter().map(|x| x.highlights).sum();
    let b: usize = a.iter().map(|x| x.bookmarks).sum();
    if h + b == 0 {
        return None;
    }
    let mut what = Vec::new();
    if h > 0 {
        what.push(ntr("{} highlight", "{} highlights", h));
    }
    if b > 0 {
        what.push(ntr("{} bookmark", "{} bookmarks", b));
    }
    let what = what.join(", ");
    let devices: BTreeSet<&str> = a.iter().map(|x| x.device.as_str()).collect();
    let device = if devices.len() == 1 {
        device_name(devices.iter().next().copied().unwrap_or(""))
    } else {
        tr("your e-readers")
    };
    let books: BTreeSet<u64> = a.iter().map(|x| x.book_id).collect();
    Some(if books.len() == 1 {
        trf("Synced {} from {} for {}", &[&what, &device, &a[0].title])
    } else {
        trf(
            "Synced {} from {} for {}",
            &[&what, &device, &ntr("{} book", "{} books", books.len())],
        )
    })
}

/// "Last synced from Kobo Libra 3 minutes ago" for the details of one book.
pub fn book_line(device: &str, time: i64, now: i64) -> String {
    trf(
        "Last synced from {} {}",
        &[&device_name(device), &relative(now, time)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    fn dev(last_seen: i64, last_sync: Option<i64>) -> Device {
        Device {
            id: "d".into(),
            name: "Kobo Libra".into(),
            client: "KOReader 2026.07.2".into(),
            plugin: "0.3.1".into(),
            last_seen,
            last_sync,
            ..Device::default()
        }
    }

    fn ev(what: What, time: i64) -> Event {
        Event {
            time,
            device_id: "d".into(),
            device: "Kobo Libra".into(),
            document: "0123456789abcdef0123456789abcdef".into(),
            title: "Building Microservices".into(),
            what,
        }
    }

    #[test]
    fn times_read_as_people_say_them() {
        assert_eq!(relative(NOW, NOW - 5), "just now");
        assert_eq!(
            relative(NOW, NOW + 500),
            "just now",
            "a clock step back is not negative"
        );
        assert_eq!(relative(NOW, NOW - 60), "1 minute ago");
        assert_eq!(relative(NOW, NOW - 300), "5 minutes ago");
        assert_eq!(relative(NOW, NOW - 7200), "2 hours ago");
        assert_eq!(relative(NOW, NOW - 86_400), "1 day ago");
        assert_eq!(relative(NOW, NOW - 3 * 86_400), "3 days ago");
    }

    #[test]
    fn a_device_is_connected_now_then_last_seen_then_never() {
        let r = device_row(&dev(NOW - 30, Some(NOW - 30)), NOW);
        assert_eq!(r.status, "Connected now");
        assert!(r.connected);
        assert_eq!(r.title, "Kobo Libra");
        assert_eq!(
            r.subtitle,
            "KOReader 2026.07.2 · plugin 0.3.1 · last sync just now"
        );
        let r = device_row(&dev(NOW - 300, Some(NOW - 180)), NOW);
        assert_eq!(r.status, "Last seen 5 minutes ago");
        assert!(!r.connected);
        assert!(r.subtitle.ends_with("last sync 3 minutes ago"));
        let r = device_row(&Device::default(), NOW);
        assert_eq!(r.status, "Never connected");
        assert_eq!(r.title, "Unknown device (KOReader)");
    }

    #[test]
    fn a_device_without_the_plugin_says_so() {
        let d = Device {
            name: UNKNOWN_DEVICE.into(),
            last_seen: NOW,
            ..Device::default()
        };
        let r = device_row(&d, NOW);
        assert!(r.subtitle.contains("no Bookx plugin"), "{}", r.subtitle);
        assert!(r.subtitle.contains("nothing synced yet"));
    }

    #[test]
    fn events_say_what_happened_in_words() {
        let doc = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            what_text(
                &What::Progress {
                    from: Some(0.12),
                    to: 0.34
                },
                doc
            ),
            "Progress 12% → 34%"
        );
        assert_eq!(
            what_text(
                &What::Progress {
                    from: None,
                    to: 0.5
                },
                doc
            ),
            "Progress 50%"
        );
        assert_eq!(
            what_text(
                &What::Received {
                    highlights: 3,
                    bookmarks: 1,
                    notes: 2,
                    removed: 0
                },
                doc
            ),
            "3 highlights added or updated, 2 with notes, 1 bookmark"
        );
        assert_eq!(
            what_text(
                &What::Received {
                    highlights: 1,
                    bookmarks: 0,
                    notes: 0,
                    removed: 2
                },
                doc
            ),
            "1 highlight added or updated, 2 removed"
        );
        assert_eq!(what_text(&What::Nothing, doc), "Nothing new");
        assert_eq!(
            what_text(
                &What::Rejected {
                    status: 422,
                    reason: "`id` is missing".into()
                },
                doc
            ),
            "Server rejected it: `id` is missing"
        );
        assert_eq!(
            what_text(&What::NotInLibrary { count: 406 }, doc),
            "Book not found in your library (hash 01234567): 406 highlights not imported"
        );
        assert_eq!(
            what_text(&What::Unplaced { count: 1 }, doc),
            "1 highlight could not be placed in the book"
        );
    }

    #[test]
    fn a_row_names_the_book_the_device_and_the_time_and_marks_problems() {
        let known = HashMap::new();
        let r = activity_row(
            &ev(
                What::Received {
                    highlights: 2,
                    bookmarks: 0,
                    notes: 0,
                    removed: 0,
                },
                NOW - 120,
            ),
            NOW,
            &known,
        );
        assert_eq!(r.title, "Building Microservices");
        assert_eq!(
            r.subtitle,
            "2 highlights added or updated · Kobo Libra · 2 minutes ago"
        );
        assert!(!r.problem);
        let r = activity_row(&ev(What::NotInLibrary { count: 3 }, NOW), NOW, &known);
        assert!(r.problem);
        // the library's title for the book wins over what the device called it
        let mut known = HashMap::new();
        known.insert(
            "0123456789abcdef0123456789abcdef".to_string(),
            "Building Microservices, 2nd Edition".to_string(),
        );
        let r = activity_row(&ev(What::Nothing, NOW), NOW, &known);
        assert_eq!(r.title, "Building Microservices, 2nd Edition");
    }

    #[test]
    fn a_book_without_a_title_is_named_by_its_short_hash() {
        let mut e = ev(
            What::Progress {
                from: None,
                to: 0.1,
            },
            NOW,
        );
        e.title.clear();
        let r = activity_row(&e, NOW, &HashMap::new());
        assert_eq!(r.title, "Unknown book (01234567)");
        let mut e = ev(
            What::Rejected {
                status: 429,
                reason: String::new(),
            },
            NOW,
        );
        e.title.clear();
        e.document.clear();
        let r = activity_row(&e, NOW, &HashMap::new());
        assert_eq!(r.title, "Request refused");
        assert!(r.subtitle.starts_with("Server rejected it (status 429)"));
    }

    #[test]
    fn the_today_line_counts_only_today() {
        let day = NOW - 3600;
        let events = vec![
            ev(
                What::Received {
                    highlights: 10,
                    bookmarks: 3,
                    notes: 0,
                    removed: 0,
                },
                NOW - 10,
            ),
            ev(
                What::Received {
                    highlights: 2,
                    bookmarks: 0,
                    notes: 1,
                    removed: 0,
                },
                NOW - 20,
            ),
            ev(
                What::Progress {
                    from: None,
                    to: 0.2,
                },
                NOW - 30,
            ),
            {
                let mut e = ev(
                    What::Progress {
                        from: Some(0.2),
                        to: 0.3,
                    },
                    NOW - 40,
                );
                e.document = "ffffffffffffffffffffffffffffffff".into();
                e
            },
            ev(
                What::Progress {
                    from: Some(0.1),
                    to: 0.2,
                },
                NOW - 50,
            ),
            ev(
                What::Received {
                    highlights: 99,
                    bookmarks: 99,
                    notes: 0,
                    removed: 0,
                },
                day - 5,
            ),
        ];
        assert_eq!(
            today_line(&events, day),
            "Today: 12 highlights, 3 bookmarks, 1 note, progress for 2 books"
        );
        assert_eq!(today_line(&[], day), "Nothing synced today");
    }

    #[test]
    fn the_empty_list_explains_what_to_do_and_where_the_server_is() {
        let t = empty_text(Some("http://192.168.1.20:8080/bookx"));
        assert!(t.contains("Nothing has synced yet"));
        assert!(t.contains("More tools, Bookx Sync, Sync now"));
        assert!(t.contains("http://192.168.1.20:8080/bookx"));
        assert!(t.contains("Test connection"));
        assert!(empty_text(None).contains("not running"));
        assert!(
            !t.contains('…') && !t.contains("..."),
            "no ellipsis in a label"
        );
    }

    #[test]
    fn the_toast_names_what_arrived_from_which_device_for_which_book() {
        let a = |id: u64, h: usize, b: usize| Arrival {
            book_id: id,
            title: format!("Book {id}"),
            device: "Kobo Libra".into(),
            highlights: h,
            bookmarks: b,
        };
        assert_eq!(
            toast_text(&[a(1, 3, 0)]).unwrap(),
            "Synced 3 highlights from Kobo Libra for Book 1"
        );
        assert_eq!(
            toast_text(&[a(1, 1, 2)]).unwrap(),
            "Synced 1 highlight, 2 bookmarks from Kobo Libra for Book 1"
        );
        assert_eq!(
            toast_text(&[a(1, 1, 0), a(2, 2, 0)]).unwrap(),
            "Synced 3 highlights from Kobo Libra for 2 books"
        );
        assert_eq!(toast_text(&[a(1, 0, 0)]), None);
        assert_eq!(toast_text(&[]), None);
    }

    #[test]
    fn the_book_line_says_who_and_when() {
        assert_eq!(
            book_line("Kobo Libra", NOW - 180, NOW),
            "Last synced from Kobo Libra 3 minutes ago"
        );
    }
}
