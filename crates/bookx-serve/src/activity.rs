//! What the e-readers do on the sync server: which devices talked to it and what they sent.
//!
//! This is the memory behind the *Sync activity* part of the Sync window. It records
//!
//! * **devices**: every device that made an authenticated request on the sync routes (`/bookx/...`),
//!   named by what the Bookx plugin sends (`X-Bookx-Device`, `X-Bookx-Device-Id`, `X-Bookx-Client`,
//!   `X-Bookx-Protocol`, or the `device` / `device_id` of the request body) and, for an old plugin or
//!   KOReader's own *Progress sync* that send nothing of the sort, by the device name of the body or
//!   as "Unknown device (KOReader)". First seen, last seen and last successful sync are kept;
//! * **events**: the newest [`MAX_EVENTS`] things that happened, newest last in the file and first when
//!   read: progress moved, annotations received, a request refused. The Bookx side adds what it could
//!   not do with them (book not in the library, quote not found), see `src/annotation_sync.rs`.
//!
//! Nothing here is secret: no password, no key and no annotation text is stored. A request that does not
//! authenticate leaves no trace (anyone on the network could otherwise fill the list). The file is small
//! (at most 50 devices and 200 events) and is written atomically after every change.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::http::Request;

/// Events kept.
pub const MAX_EVENTS: usize = 200;
/// Devices kept (the one seen longest ago goes first).
pub const MAX_DEVICES: usize = 50;
/// A device that was seen this recently counts as connected.
pub const CONNECTED_SECS: i64 = 120;
const MAX_TEXT: usize = 128;

/// The name shown for a device that did not say what it is.
pub const UNKNOWN_DEVICE: &str = "Unknown device (KOReader)";

/// One device that talked to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Device {
    /// Stable id the device sent, else its name.
    pub id: String,
    /// What the device calls itself (KOReader's `Device.model` or the name typed in the plugin),
    /// as it arrived. Shown through [`Device::display_name`].
    pub name: String,
    /// The name the user gave this device in Bookx. Empty: none. A later request never changes it.
    pub alias: String,
    pub user: String,
    /// `KOReader 2026.07.2`, empty when the client did not say.
    pub client: String,
    /// Plugin version (`0.3.1`), empty for a client that is not the Bookx plugin.
    pub plugin: String,
    /// Protocol number the client announced, `0` when unknown.
    pub protocol: u32,
    pub first_seen: i64,
    pub last_seen: i64,
    /// The last time annotations or progress were stored successfully.
    pub last_sync: Option<i64>,
    pub requests: u64,
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum What {
    /// Reading position moved (`from` is `None` for the first one the server hears of).
    Progress { from: Option<f64>, to: f64 },
    /// New or changed annotations were stored.
    Received {
        highlights: u32,
        bookmarks: u32,
        notes: u32,
        removed: u32,
    },
    /// The device synced and had nothing new.
    Nothing,
    /// The server refused a request.
    Rejected { status: u16, reason: String },
    /// Bookx side: annotations for a book that is not in the library (matched by neither file nor title).
    NotInLibrary { count: u32 },
    /// Bookx side: highlights whose text was not found in the book; kept as notes without a place.
    Unplaced { count: u32 },
}

/// One entry of the activity list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub time: i64,
    pub device_id: String,
    pub device: String,
    /// The 32 hex digits KOReader names the book by (may be empty).
    pub document: String,
    /// The title the device sent (may be empty: progress sync sends none).
    pub title: String,
    pub what: What,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct State {
    /// Changes so far; the UI compares it to know when to redraw.
    generation: u64,
    devices: Vec<Device>,
    /// Oldest first.
    events: VecDeque<Event>,
}

/// Who is asking, as far as the request says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub id: String,
    pub client: String,
    pub plugin: String,
    pub protocol: u32,
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT)
        .collect::<String>()
        .trim()
        .to_string()
}

impl Ident {
    /// Reads `X-Bookx-Device`, `X-Bookx-Device-Id`, `X-Bookx-Client` (`KOReader/2026.07.2 bookx-plugin/0.3.1`)
    /// and `X-Bookx-Protocol`. Every value is optional, cleaned and cut to 128 characters.
    pub fn from_request(req: &Request) -> Ident {
        let h = |n: &str| req.header(n).map(clean).unwrap_or_default();
        let mut id = Ident {
            name: h("x-bookx-device"),
            id: h("x-bookx-device-id"),
            protocol: h("x-bookx-protocol").parse().unwrap_or(0),
            ..Ident::default()
        };
        for part in h("x-bookx-client").split_whitespace() {
            if let Some((product, version)) = part.split_once('/') {
                match product.to_ascii_lowercase().as_str() {
                    "koreader" => id.client = format!("KOReader {version}"),
                    "bookx-plugin" => id.plugin = version.to_string(),
                    _ => {}
                }
            }
        }
        id
    }

    /// The body of a request may name the device too (the plugin always does): it wins over the
    /// header for the name because it is complete UTF-8, a header is ASCII.
    pub fn with_body(mut self, name: &str, id: &str) -> Ident {
        let (name, id) = (clean(name), clean(id));
        if !name.is_empty() {
            self.name = name;
        }
        if !id.is_empty() {
            self.id = id;
        }
        self
    }

    fn key(&self) -> String {
        if !self.id.is_empty() {
            self.id.clone()
        } else if !self.name.is_empty() {
            self.name.clone()
        } else {
            UNKNOWN_DEVICE.to_string()
        }
    }
}

/// The activity store (shared between the sync routes and the app).
pub struct ActivityLog {
    state: Mutex<State>,
    path: Option<PathBuf>,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
    /// document -> whether the Bookx library has that book (set by the app after a merge; not saved).
    matches: Mutex<std::collections::HashMap<String, bool>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl ActivityLog {
    /// In memory only (tests).
    pub fn memory() -> ActivityLog {
        ActivityLog {
            state: Mutex::new(State::default()),
            path: None,
            clock: Box::new(|| chrono::Utc::now().timestamp()),
            matches: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// The app says whether `document` is a book of the library; the next answer to the device carries it.
    pub fn set_in_library(&self, document: &str, found: bool) {
        lock(&self.matches).insert(document.to_string(), found);
    }

    /// `None` until the app has looked at the book.
    pub fn in_library(&self, document: &str) -> Option<bool> {
        lock(&self.matches).get(document).copied()
    }

    /// Kept in a JSON file. A missing, empty or damaged file starts empty (it is only a log).
    pub fn open(path: &Path) -> Result<ActivityLog> {
        let state = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Ok(ActivityLog {
            state: Mutex::new(state),
            path: Some(path.to_path_buf()),
            ..ActivityLog::memory()
        })
    }

    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> ActivityLog {
        self.clock = Box::new(clock);
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn save(&self, st: &State) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(st)?)?;
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
    }

    fn commit(&self, st: &mut State) {
        st.generation += 1;
        let _ = self.save(st);
    }

    /// A device made an authenticated request. `synced` marks one that stored something.
    /// Returns the device id (its key).
    pub fn touch(&self, who: &Ident, user: &str, synced: bool) -> String {
        let now = self.now();
        let key = who.key();
        let mut st = lock(&self.state);
        let at = match st.devices.iter().position(|d| d.id == key) {
            Some(i) => i,
            None => {
                if st.devices.len() >= MAX_DEVICES {
                    if let Some(old) = st
                        .devices
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, d)| d.last_seen)
                        .map(|(i, _)| i)
                    {
                        st.devices.remove(old);
                    }
                }
                st.devices.push(Device {
                    id: key.clone(),
                    first_seen: now,
                    ..Device::default()
                });
                st.devices.len() - 1
            }
        };
        let d = &mut st.devices[at];
        // a later request that knows more fills in what an earlier one did not say
        if !who.name.is_empty() {
            d.name = who.name.clone();
        } else if d.name.is_empty() {
            d.name = UNKNOWN_DEVICE.to_string();
        }
        if !who.client.is_empty() {
            d.client = who.client.clone();
        }
        if !who.plugin.is_empty() {
            d.plugin = who.plugin.clone();
        }
        if who.protocol > 0 {
            d.protocol = who.protocol;
        }
        d.user = clean(user);
        d.last_seen = now;
        d.requests += 1;
        if synced {
            d.last_sync = Some(now);
        }
        self.commit(&mut st);
        key
    }

    /// Adds an event. A repeated "nothing new" of the same device and book only moves the time of
    /// the last one, so a device that syncs on every page does not push everything else out.
    pub fn record(&self, mut ev: Event) {
        ev.time = if ev.time > 0 { ev.time } else { self.now() };
        ev.device = clean(&ev.device);
        ev.title = clean(&ev.title);
        let mut st = lock(&self.state);
        if ev.what == What::Nothing {
            if let Some(last) = st.events.back_mut() {
                if last.what == What::Nothing
                    && last.device_id == ev.device_id
                    && last.document == ev.document
                {
                    last.time = ev.time;
                    self.commit(&mut st);
                    return;
                }
            }
        }
        st.events.push_back(ev);
        while st.events.len() > MAX_EVENTS {
            st.events.pop_front();
        }
        self.commit(&mut st);
    }

    /// Convenience for a device that was just touched.
    pub fn record_for(&self, who: &Ident, document: &str, title: &str, what: What) {
        self.record(Event {
            time: 0,
            device_id: who.key(),
            device: if who.name.is_empty() {
                UNKNOWN_DEVICE.to_string()
            } else {
                who.name.clone()
            },
            document: document.to_string(),
            title: title.to_string(),
            what,
        });
    }

    /// The devices, the one seen last first.
    pub fn devices(&self) -> Vec<Device> {
        let mut v = lock(&self.state).devices.clone();
        v.sort_by_key(|d| std::cmp::Reverse(d.last_seen));
        v
    }

    /// The newest `limit` events, newest first.
    pub fn events(&self, limit: usize) -> Vec<Event> {
        lock(&self.state)
            .events
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Changes so far (never goes down while the log lives).
    pub fn generation(&self) -> u64 {
        lock(&self.state).generation
    }

    /// Gives a device a name of the user's own (`""` takes it back). `false` when no device has
    /// that id. The name is cleaned like every name that arrives and survives later requests.
    pub fn rename(&self, id: &str, name: &str) -> bool {
        let mut st = lock(&self.state);
        let Some(d) = st.devices.iter_mut().find(|d| d.id == id) else {
            return false;
        };
        d.alias = clean(name);
        self.commit(&mut st);
        true
    }

    /// The name to show for the device of an event: the user's name for it, else the friendly
    /// name of what the event says.
    pub fn name_for(&self, device_id: &str, reported: &str) -> String {
        let alias = lock(&self.state)
            .devices
            .iter()
            .find(|d| d.id == device_id && !d.alias.is_empty())
            .map(|d| d.alias.clone());
        alias.unwrap_or_else(|| crate::device_names::friendly_name(reported))
    }

    /// Forgets devices and events (the *Clear* button).
    pub fn clear(&self) {
        let mut st = lock(&self.state);
        st.devices.clear();
        st.events.clear();
        self.commit(&mut st);
    }
}

/// How a device is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Seen within [`CONNECTED_SECS`].
    Connected,
    /// Seen before, `seconds` ago.
    Seen { seconds: i64 },
}

impl Device {
    /// The name for people: the one the user gave, else the marketing name of the model it
    /// reported (`Onyx Scubapilot-IDP` is `Boox Leaf 2`), else what it reported, tidied.
    pub fn display_name(&self) -> String {
        if !self.alias.is_empty() {
            return self.alias.clone();
        }
        crate::device_names::friendly_name(&self.name)
    }

    pub fn presence(&self, now: i64) -> Presence {
        let ago = (now - self.last_seen).max(0);
        if ago <= CONNECTED_SECS {
            Presence::Connected
        } else {
            Presence::Seen { seconds: ago }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    fn clocked() -> (ActivityLog, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(1_700_000_000));
        let c = t.clone();
        (
            ActivityLog::memory().with_clock(move || c.load(Ordering::SeqCst)),
            t,
        )
    }

    fn who(name: &str, id: &str) -> Ident {
        Ident {
            name: name.into(),
            id: id.into(),
            ..Ident::default()
        }
    }

    #[test]
    fn a_device_is_remembered_with_first_and_last_seen_and_filled_in_by_later_requests() {
        let (log, t) = clocked();
        log.touch(&who("", "abc"), "u", false);
        assert_eq!(log.devices()[0].name, UNKNOWN_DEVICE);
        t.fetch_add(60, Ordering::SeqCst);
        let mut w = who("Kobo Libra", "abc");
        w.client = "KOReader 2026.07.2".into();
        w.plugin = "0.3.1".into();
        w.protocol = 1;
        log.touch(&w, "u", true);
        let d = log.devices();
        assert_eq!(d.len(), 1, "same id, one device");
        assert_eq!(d[0].name, "Kobo Libra");
        assert_eq!(d[0].client, "KOReader 2026.07.2");
        assert_eq!(d[0].plugin, "0.3.1");
        assert_eq!(d[0].first_seen, 1_700_000_000);
        assert_eq!(d[0].last_seen, 1_700_000_060);
        assert_eq!(d[0].last_sync, Some(1_700_000_060));
        assert_eq!(d[0].requests, 2);
        // a later request that says nothing keeps what is known
        log.touch(&who("", "abc"), "u", false);
        assert_eq!(log.devices()[0].name, "Kobo Libra");
        assert_eq!(log.devices()[0].last_sync, Some(1_700_000_060));
    }

    #[test]
    fn an_old_plugin_that_sends_no_headers_is_an_unknown_device_and_one_row() {
        let (log, _) = clocked();
        log.touch(&Ident::default(), "u", true);
        log.touch(&Ident::default(), "u", true);
        let d = log.devices();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, UNKNOWN_DEVICE);
    }

    #[test]
    fn the_headers_are_read_cleaned_and_limited() {
        let mut r = Request::new("GET", "/annotations/x");
        r.headers = vec![
            ("x-bookx-device".into(), "Kobo\u{7}Libra".into()),
            ("x-bookx-device-id".into(), "a".repeat(500)),
            (
                "x-bookx-client".into(),
                "KOReader/2026.07.2 bookx-plugin/0.3.1 other/9".into(),
            ),
            ("x-bookx-protocol".into(), "1".into()),
        ];
        let i = Ident::from_request(&r);
        assert_eq!(i.name, "KoboLibra", "control characters are dropped");
        assert_eq!(i.id.chars().count(), 128);
        assert_eq!(i.client, "KOReader 2026.07.2");
        assert_eq!(i.plugin, "0.3.1");
        assert_eq!(i.protocol, 1);
        let none = Ident::from_request(&Request::new("GET", "/x"));
        assert_eq!(none, Ident::default());
        let r2 = {
            let mut r = Request::new("GET", "/x");
            r.headers = vec![("x-bookx-protocol".into(), "nope".into())];
            r
        };
        assert_eq!(Ident::from_request(&r2).protocol, 0);
    }

    #[test]
    fn the_body_name_beats_the_ascii_header() {
        let i = Ident {
            name: "Kobo Libra".into(),
            ..Ident::default()
        }
        .with_body("Kobo Libra 日本", "dev-1");
        assert_eq!(i.name, "Kobo Libra 日本");
        assert_eq!(i.id, "dev-1");
        let j = i.clone().with_body("", "");
        assert_eq!(j, i, "an empty body field changes nothing");
    }

    #[test]
    fn events_are_capped_and_listed_newest_first() {
        let (log, t) = clocked();
        for n in 0..(MAX_EVENTS + 25) {
            t.fetch_add(1, Ordering::SeqCst);
            log.record_for(
                &who("Kobo", "k"),
                "",
                &format!("Book {n}"),
                What::Received {
                    highlights: 1,
                    bookmarks: 0,
                    notes: 0,
                    removed: 0,
                },
            );
        }
        let e = log.events(1000);
        assert_eq!(e.len(), MAX_EVENTS);
        assert_eq!(
            e[0].title,
            format!("Book {}", MAX_EVENTS + 24),
            "newest first"
        );
        assert_eq!(e[MAX_EVENTS - 1].title, "Book 25", "the oldest 25 are gone");
        assert!(e.windows(2).all(|w| w[0].time >= w[1].time));
        assert_eq!(log.events(3).len(), 3);
    }

    #[test]
    fn a_repeated_nothing_new_does_not_push_other_events_out() {
        let (log, t) = clocked();
        let k = who("Kobo", "k");
        log.record_for(
            &k,
            "d",
            "T",
            What::Progress {
                from: None,
                to: 0.1,
            },
        );
        for _ in 0..5 {
            t.fetch_add(10, Ordering::SeqCst);
            log.record_for(&k, "d", "T", What::Nothing);
        }
        let e = log.events(10);
        assert_eq!(e.len(), 2, "one progress and one nothing-new");
        assert_eq!(
            e[0].time, 1_700_000_050,
            "the nothing-new carries the latest time"
        );
    }

    #[test]
    fn devices_are_capped_and_the_longest_unseen_goes_first() {
        let (log, t) = clocked();
        for n in 0..(MAX_DEVICES + 3) {
            t.fetch_add(1, Ordering::SeqCst);
            log.touch(&who(&format!("dev {n}"), &format!("id{n}")), "u", false);
        }
        let d = log.devices();
        assert_eq!(d.len(), MAX_DEVICES);
        assert!(d.iter().all(|x| x.name != "dev 0"));
        assert_eq!(
            d[0].name,
            format!("dev {}", MAX_DEVICES + 2),
            "last seen first"
        );
    }

    #[test]
    fn a_boox_is_shown_by_its_marketing_name_and_keeps_what_it_reported() {
        let (log, _) = clocked();
        log.touch(&who("Onyx Scubapilot-IDP", "boox1"), "u", true);
        let d = &log.devices()[0];
        assert_eq!(d.name, "Onyx Scubapilot-IDP", "what arrived is kept");
        assert_eq!(d.display_name(), "Boox Leaf 2");
        assert_eq!(log.name_for("boox1", "Onyx Scubapilot-IDP"), "Boox Leaf 2");
    }

    #[test]
    fn a_name_the_user_gave_wins_and_survives_the_next_request() {
        let (log, _) = clocked();
        log.touch(&who("Onyx Scubapilot-IDP", "boox1"), "u", true);
        assert!(!log.rename("nobody", "x"));
        assert!(log.rename("boox1", "  Anna\u{7}'s Leaf  "));
        assert_eq!(log.devices()[0].display_name(), "Anna's Leaf");
        assert_eq!(log.name_for("boox1", "Onyx Scubapilot-IDP"), "Anna's Leaf");
        log.touch(&who("Onyx Scubapilot-IDP", "boox1"), "u", true);
        assert_eq!(log.devices()[0].display_name(), "Anna's Leaf");
        assert!(log.rename("boox1", ""));
        assert_eq!(
            log.devices()[0].display_name(),
            "Boox Leaf 2",
            "empty takes it back"
        );
    }

    #[test]
    fn a_name_the_user_gave_is_remembered_across_a_restart() {
        let dir = std::env::temp_dir().join(format!("bookx-activity-alias-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sync-activity.json");
        {
            let log = ActivityLog::open(&path).unwrap();
            log.touch(&who("leaf2", "b"), "u", true);
            assert!(log.rename("b", "Bedroom"));
        }
        let again = ActivityLog::open(&path).unwrap();
        assert_eq!(again.devices()[0].display_name(), "Bedroom");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn presence_is_connected_for_two_minutes() {
        let d = Device {
            last_seen: 1000,
            ..Device::default()
        };
        assert_eq!(d.presence(1000 + CONNECTED_SECS), Presence::Connected);
        assert_eq!(
            d.presence(1000 + CONNECTED_SECS + 1),
            Presence::Seen {
                seconds: CONNECTED_SECS + 1
            }
        );
        assert_eq!(
            d.presence(900),
            Presence::Connected,
            "a clock step back is not an error"
        );
    }

    #[test]
    fn it_survives_a_restart_and_a_damaged_file_starts_empty() {
        let dir = std::env::temp_dir().join(format!("bookx-activity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sync-activity.json");
        {
            let log = ActivityLog::open(&path).unwrap();
            log.touch(&who("Kobo", "k"), "u", true);
            log.record_for(&who("Kobo", "k"), "d", "T", What::NotInLibrary { count: 2 });
        }
        let again = ActivityLog::open(&path).unwrap();
        assert_eq!(again.devices().len(), 1);
        assert_eq!(again.events(5)[0].what, What::NotInLibrary { count: 2 });
        assert!(again.generation() >= 2);
        std::fs::write(&path, "{ not json").unwrap();
        let broken = ActivityLog::open(&path).unwrap();
        assert!(broken.devices().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_library_match_is_unknown_until_the_app_says() {
        let (log, _) = clocked();
        assert_eq!(log.in_library("d"), None);
        log.set_in_library("d", false);
        assert_eq!(log.in_library("d"), Some(false));
        log.set_in_library("d", true);
        assert_eq!(log.in_library("d"), Some(true));
    }

    #[test]
    fn clear_forgets_everything_and_moves_the_generation() {
        let (log, _) = clocked();
        log.touch(&who("Kobo", "k"), "u", true);
        log.record_for(&who("Kobo", "k"), "", "", What::Nothing);
        let g = log.generation();
        log.clear();
        assert!(log.devices().is_empty() && log.events(5).is_empty());
        assert!(log.generation() > g);
    }
}
