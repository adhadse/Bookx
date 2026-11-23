//! The state of every feature as report facts: what is configured and what is running, without a value
//! the user typed. Each function takes the plain data it needs and returns a [`Section`], so a test
//! builds the state by hand and the replay corpus (`diag_replay`) uses the same functions as the app.
//!
//! Adding a feature: write one function here (or register a provider with [`crate::trace::register`])
//! that returns `Fact`s, and add its call to [`sections`]. The kind of `Fact` decides what may be shown;
//! see `diag_facts`. No GTK.

use crate::config::Config;
use crate::diag_facts::{Fact, Section};

/// Plain data about the running built-in server (the app reads it from `Sharing`).
#[derive(Debug, Clone, Default)]
pub struct ServerView {
    pub running: bool,
    pub devices: Vec<SeenDevice>,
    /// `(HTTP status, count)` of the requests the server refused lately.
    pub rejected: Vec<(u16, usize)>,
    pub events: usize,
}

/// One device that talked to the built-in server. `reported` is what the device calls itself (a model
/// code or a name somebody typed): it is shown only through [`crate::trace::device_label`].
#[derive(Debug, Clone, Default)]
pub struct SeenDevice {
    pub reported: String,
    /// `KOReader 2026.07.2`
    pub client: String,
    pub plugin: String,
    pub protocol: u32,
    pub requests: u64,
    /// Seconds since it last synced successfully, `None` when never.
    pub last_sync_ago: Option<i64>,
    pub last_seen_ago: i64,
}

/// A class of "how long ago": the exact moment of someone's sync says when they read.
pub fn ago_class(secs: i64) -> &'static str {
    match secs {
        s if s < 0 => "unknown",
        s if s < 300 => "under 5 minutes",
        s if s < 3600 => "under an hour",
        s if s < 86_400 => "under a day",
        s if s < 7 * 86_400 => "under a week",
        _ => "over a week",
    }
}

const AGO_CLASSES: &[&str] = &[
    "unknown",
    "under 5 minutes",
    "under an hour",
    "under a day",
    "under a week",
    "over a week",
];

fn ago(secs: i64) -> Fact {
    Fact::choice(ago_class(secs), AGO_CLASSES)
}

/// The version number after the program name in `KOReader 2026.07.2`.
pub fn client_version(client: &str) -> String {
    client.split_whitespace().last().unwrap_or("").to_string()
}

pub fn sync_client(cfg: &Config) -> Section {
    let k = &cfg.kosync;
    let mut s = Section::new("Sync (KOSync client)");
    s.row("Enabled", Fact::onoff(k.enabled))
        .row("Server address", Fact::address(&k.server))
        .row(
            "Document matching",
            Fact::word(if k.by_filename {
                "filename"
            } else {
                "partial-md5"
            }),
        )
        .row("User name set", Fact::flag(!k.user.trim().is_empty()))
        .row("Password set", Fact::flag(!k.password.is_empty()))
        .row("Logged in", Fact::flag(k.logged_in))
        .row(
            "Device name set by the user",
            Fact::flag(!k.device_name.trim().is_empty()),
        );
    s
}

pub fn server(cfg: &Config, view: &ServerView) -> Section {
    let c = &cfg.share;
    let mut s = Section::new("Built-in server");
    s.row("Running", Fact::flag(view.running))
        .row("Port", Fact::num(c.port as u64))
        .row("Reachable from other devices", Fact::flag(c.lan))
        .row("HTTPS", Fact::flag(c.https))
        .row("Own certificate", Fact::flag(!c.cert_file.is_empty()))
        .row("Reading positions (KOSync)", Fact::onoff(c.kosync))
        .row("Annotations from KOReader", Fact::onoff(c.annotations))
        .row(
            "Behind a reverse proxy",
            Fact::flag(!c.public_prefix.is_empty() || !c.trusted_proxies.is_empty()),
        )
        .row(
            "Extra host names allowed",
            Fact::count(
                c.allowed_hosts
                    .split([',', ' ', ';'])
                    .filter(|h| !h.is_empty())
                    .count(),
            ),
        )
        .row("Extra accounts", Fact::count(c.accounts.len()))
        .row("Restored at launch", Fact::flag(c.running));
    if !view.rejected.is_empty() {
        s.row(
            "Requests the server refused",
            Fact::statuses(&view.rejected),
        );
    }
    for (i, d) in view.devices.iter().take(5).enumerate() {
        let n = i + 1;
        s.row(
            &format!("Device {n}"),
            Fact::text(&crate::trace::device_label(&d.reported)),
        )
        .row(
            &format!("Device {n} client"),
            Fact::version(&client_version(&d.client)),
        )
        .row(&format!("Device {n} plugin"), Fact::version(&d.plugin))
        .row(
            &format!("Device {n} protocol"),
            Fact::num(d.protocol as u64),
        )
        .row(
            &format!("Device {n} requests"),
            Fact::count_class(d.requests),
        )
        .row(&format!("Device {n} last seen"), ago(d.last_seen_ago))
        .row(
            &format!("Device {n} last successful sync"),
            match d.last_sync_ago {
                Some(a) => ago(a),
                None => Fact::word("never"),
            },
        );
    }
    s
}

/// Helper programs that features need: found or not (never where).
#[derive(Debug, Clone, Copy, Default)]
pub struct Helpers {
    pub rar: bool,
    pub djvu: bool,
    pub espeak: bool,
    pub audio_player: bool,
}

impl Helpers {
    pub fn detect() -> Helpers {
        Helpers {
            rar: bookx_formats::rar::find_tool().is_some(),
            djvu: bookx_formats::djvu::find_tools().is_some(),
            espeak: bookx_tts::phonemize::espeak_program().is_some(),
            audio_player: bookx_tts::sink::CommandSink::available(),
        }
    }
}

pub fn helpers(h: &Helpers) -> Section {
    let mut s = Section::new("Conversion");
    s.row("RAR helper (CBR): unrar or 7-Zip", Fact::flag(h.rar))
        .row("DjVuLibre (DJVU)", Fact::flag(h.djvu));
    s
}

pub fn tts(cfg: &Config, h: &Helpers) -> Section {
    let mut s = Section::new("Read aloud");
    s.row("Engine", Fact::text(&cfg.tts.engine))
        .row("espeak-ng found", Fact::flag(h.espeak))
        .row(
            "Audio player (pacat, pw-cat or aplay) found",
            Fact::flag(h.audio_player),
        );
    s
}

pub fn backups(cfg: &Config) -> Section {
    let b = &cfg.health.backup;
    let mut s = Section::new("Backups");
    s.row("Automatic backup", Fact::onoff(cfg.health.auto_backup));
    s.row("Places", Fact::count(b.destinations.len()));
    for (i, d) in b.destinations.iter().take(6).enumerate() {
        let kind = format!("{:?}", d.kind).to_ascii_lowercase();
        s.row(
            &format!("Place {}", i + 1),
            Fact::text(&format!(
                "{kind}, encrypted {}, books {}",
                if d.encrypt { "yes" } else { "no" },
                if d.include_books { "yes" } else { "no" }
            )),
        );
        if d.kind != bookx_backup::DestKind::Folder {
            s.row(
                &format!("Place {} address", i + 1),
                Fact::address(&d.location),
            );
        }
    }
    s
}

pub fn network(cfg: &Config) -> Section {
    let mut s = Section::new("Network");
    s.row("Offline mode", Fact::onoff(cfg.offline));
    s.row("OPDS catalogues", Fact::count(cfg.opds_catalogs.len()));
    for (i, c) in cfg.opds_catalogs.iter().take(4).enumerate() {
        s.row(
            &format!("OPDS catalogue {} address", i + 1),
            Fact::address(&c.url),
        );
    }
    s.row("News sites", Fact::count(cfg.news_sites.len()));
    s
}

pub fn settings(cfg: &Config, language: &str) -> Section {
    let mut s = Section::new("Display and interface");
    s.row("Interface language", Fact::lang(language))
        .row(
            "Language script",
            Fact::choice(
                crate::diag_system::script_of(language),
                &[
                    "right-to-left (Arabic script)",
                    "right-to-left (Hebrew script)",
                    "Devanagari",
                    "Bengali",
                    "other Indic or South-East Asian",
                    "CJK",
                    "Cyrillic",
                    "Greek",
                    "Latin",
                    "unknown",
                ],
            ),
        )
        .row("E-ink mode", Fact::onoff(cfg.eink))
        .row(
            "Reader font size class",
            Fact::choice(
                font_class(cfg.reader.font_size),
                &["small", "normal", "large", "very large"],
            ),
        )
        .row(
            "Paged or scrolling",
            Fact::word(
                if format!("{:?}", cfg.reader.layout)
                    .to_ascii_lowercase()
                    .contains("scroll")
                {
                    "scrolling"
                } else {
                    "paged or other"
                },
            ),
        )
        .row("Hyphenation", Fact::onoff(cfg.reader.hyphenate))
        .row(
            "Optimal line breaks",
            Fact::onoff(cfg.reader.optimal_breaks),
        )
        .row("Publisher styles", Fact::onoff(cfg.reader.publisher_styles))
        .row(
            "Window size class",
            Fact::choice(
                window_class(cfg.window_width, cfg.window_height),
                &["small", "medium", "large"],
            ),
        )
        .row("Saved searches", Fact::count(cfg.saved_searches.len()));
    s
}

fn font_class(px: f64) -> &'static str {
    match px {
        p if p < 14.0 => "small",
        p if p < 22.0 => "normal",
        p if p < 32.0 => "large",
        _ => "very large",
    }
}

fn window_class(w: i32, h: i32) -> &'static str {
    match w.max(h) {
        m if m < 1000 => "small",
        m if m < 1700 => "medium",
        _ => "large",
    }
}

/// Plugins and patches by id and version.
pub fn plugins(list: &[bookx_plugin::PluginStatus], patches: usize) -> Section {
    let mut s = Section::new("Plugins and patches");
    s.row("Plugins", Fact::count(list.len()))
        .row("Patches", Fact::count(patches));
    for (i, p) in list.iter().take(12).enumerate() {
        let state = if let Some(e) = &p.error {
            format!("failed to load ({})", crate::diag_addr::clean_text(e))
        } else if p.enabled {
            "enabled".to_string()
        } else {
            "disabled".to_string()
        };
        s.row(
            &format!("Plugin {}", i + 1),
            Fact::text(&format!("{} {} {state}", p.id, p.version)),
        );
    }
    s
}

/// The book read last: its format, size class and progress.
pub fn last_book(lib: &crate::core::library::Library) -> Section {
    let mut s = Section::new("Reader");
    let last = lib
        .books
        .iter()
        .filter(|b| b.last_read.is_some())
        .max_by_key(|b| b.last_read);
    match last {
        Some(b) => {
            let ext = b.file.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            s.row("Last book read, format", Fact::choice(&ext, BOOK_FORMATS))
                .row("Last book read, size", Fact::size(b.size))
                .row("Last book read, progress", Fact::ratio(b.progress))
                .row(
                    "Last book read, highlights stored",
                    Fact::count(b.highlights.len()),
                )
                .row("Last book read, bookmarks", Fact::count(b.bookmarks.len()))
                .row(
                    "Last book read, formats in the library entry",
                    Fact::count(b.formats.len().max(1)),
                )
                .row(
                    "Last book read, copy protected",
                    Fact::flag(!b.drm.is_empty()),
                )
                .row("Last book read, language", Fact::lang(&b.meta.language));
        }
        None => {
            s.row("Last book read", Fact::word("none yet"));
        }
    }
    s
}

pub const BOOK_FORMATS: &[&str] = &[
    "epub", "kepub", "pdf", "mobi", "azw", "azw3", "fb2", "fbz", "cbz", "cbr", "cbt", "cb7",
    "djvu", "djv", "txt", "md", "docx", "doc", "odt", "rtf", "html", "htm", "htmlz", "txtz", "lit",
    "lrf", "pdb", "prc", "mp3", "m4b", "m4a", "ogg", "opus", "flac", "wav", "zip",
];

/// A device that is connected now (read from the drivers when the report is opened from the Devices
/// window).
#[derive(Debug, Clone, Default)]
pub struct ConnectedDevice {
    /// What the device calls itself.
    pub reported: String,
    /// The driver's id: `mtp:...`, `webdav:...`, or a mount.
    pub id: String,
    pub mounted: bool,
    pub free: Option<(u64, u64)>,
}

/// The rows of the *Devices* section: how many, and each by model, connection and state.
pub fn device_rows(list: &[ConnectedDevice]) -> Vec<(String, Fact)> {
    let mut rows = vec![("Connected devices".to_string(), Fact::count(list.len()))];
    for (i, d) in list.iter().take(6).enumerate() {
        let n = i + 1;
        let backend = if d.id.starts_with("mtp:") {
            "mtp"
        } else if d.id.starts_with("webdav:") {
            "cloud"
        } else {
            "usb"
        };
        rows.push((
            format!("Device {n}"),
            Fact::text(&crate::trace::device_label(&d.reported)),
        ));
        rows.push((format!("Device {n} connection"), Fact::word(backend)));
        rows.push((format!("Device {n} mounted"), Fact::flag(d.mounted)));
        if let Some((a, t)) = d.free {
            let (c, nearly) = crate::diag_system::free_class(a, t);
            rows.push((
                format!("Device {n} free space"),
                Fact::choice(
                    c,
                    &[
                        "under 200 MB",
                        "200 MB-1 GB",
                        "1-10 GB",
                        "10-100 GB",
                        "over 100 GB",
                    ],
                ),
            ));
            rows.push((format!("Device {n} nearly full"), Fact::flag(nearly)));
        }
    }
    rows
}

/// The books the trace names (by id), as facts. Titles and authors are `Fact::title`: shown only when
/// the user allowed them.
pub fn books_involved(lib: &crate::core::library::Library, ids: &[u64]) -> Section {
    let mut s = Section::new("Books involved");
    for (n, id) in ids.iter().take(5).enumerate() {
        let n = n + 1;
        match lib.books.iter().find(|b| b.id == *id) {
            Some(b) => {
                let ext = b.file.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                s.row(
                    &format!("Book {n} format"),
                    Fact::choice(&ext, BOOK_FORMATS),
                )
                .row(&format!("Book {n} size"), Fact::size(b.size))
                .row(&format!("Book {n} progress"), Fact::ratio(b.progress))
                .row(&format!("Book {n} in the library"), Fact::flag(true))
                .row(&format!("Book {n} title"), Fact::title(&b.meta.title))
                .row(
                    &format!("Book {n} authors"),
                    Fact::title(&b.meta.authors.join(", ")),
                );
            }
            None => {
                s.row(&format!("Book {n} in the library"), Fact::flag(false));
            }
        }
    }
    s
}

/// The ids of the books the trace is about, newest first, without doubles.
pub fn book_ids(entries: &[crate::trace::Entry]) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for e in entries.iter().rev() {
        if let Some(id) = e.field("book").and_then(|v| v.parse().ok()) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// Everything the report reads, as plain data: the app fills it from its state, the tests with a fake
/// world. [`input`] turns it into the report's [`crate::report::Input`].
pub struct World<'a> {
    pub env: crate::report::Environment,
    pub cfg: &'a Config,
    pub lib: &'a crate::core::library::Library,
    pub indexed: Option<(usize, usize)>,
    pub plugins: Vec<bookx_plugin::PluginStatus>,
    pub patches: usize,
    pub keyring: String,
    pub server: ServerView,
    pub jobs: Vec<crate::report::JobLine>,
    pub log: Vec<String>,
    pub trace: Vec<crate::trace::Entry>,
    pub previous: Vec<crate::trace::Entry>,
    pub crash: Option<String>,
    pub flatpak_info: Option<String>,
    pub backend: String,
    pub renderer: String,
    pub vars: std::collections::BTreeMap<String, String>,
    pub extra: Vec<(crate::trace::Area, Vec<(String, Fact)>)>,
    pub entry: Option<String>,
    /// The library folder as read; `None` reads the real one.
    pub probe: Option<crate::diag_system::LibraryProbe>,
    pub helpers: Helpers,
}

/// The report input of a world.
pub fn input(w: &World) -> crate::report::Input {
    let var = |k: &str| w.vars.get(k).cloned().unwrap_or_default();
    let mut secs = vec![
        crate::diag_system::machine_section(&w.backend, &w.renderer, &var),
        crate::diag_system::process_section(Some(*crate::applog::start_instant())),
        crate::diag_system::library_fs_section(
            &w.probe
                .clone()
                .unwrap_or_else(|| crate::diag_system::probe(&w.lib.root)),
        ),
    ];
    if let Some(t) = &w.flatpak_info {
        secs.push(crate::diag_system::flatpak_section(t));
    }
    secs.extend(sections(
        w.cfg,
        &w.env.language,
        &w.server,
        &w.plugins,
        w.patches,
        &w.helpers,
    ));
    secs.push(last_book(w.lib));
    secs.push(books_involved(w.lib, &book_ids(&w.trace)));
    for (area, rows) in &w.extra {
        let mut s = Section::new(area.label());
        s.rows = rows.clone();
        secs.push(s);
    }
    crate::report::Input {
        env: w.env.clone(),
        settings: crate::report::summarize_with(
            w.cfg,
            w.plugins
                .iter()
                .filter(|p| !p.id.is_empty())
                .map(|p| p.id.clone())
                .collect(),
            w.patches,
            &w.keyring,
        ),
        library: Some(crate::report::library_stats(w.lib, w.indexed)),
        jobs: w.jobs.clone(),
        log: w.log.clone(),
        crash: w.crash.clone(),
        sections: secs,
        trace: w.trace.clone(),
        previous: w.previous.clone(),
        entry: w.entry.clone(),
    }
}

/// The scrub rules for the report: the debug-log rules plus the names of devices and this machine.
pub fn scrub_rules(
    cfg: &Config,
    lib: &crate::core::library::Library,
    device_names: Vec<String>,
    hostname: &str,
) -> crate::debug_log::Scrub {
    let mut s = crate::debug_log::scrub_for(cfg, lib);
    s.accounts
        .extend(device_names.into_iter().filter(|n| !n.trim().is_empty()));
    let own = cfg.kosync.device_name.trim();
    if !own.is_empty() {
        s.accounts.push(own.to_string());
    }
    let h = hostname.trim();
    if h.len() >= 3 && h != "localhost" {
        s.hosts.push(h.to_string());
    }
    s
}

/// Every section from the config and the running server.
pub fn sections(
    cfg: &Config,
    language: &str,
    view: &ServerView,
    plugins_list: &[bookx_plugin::PluginStatus],
    patches: usize,
    h: &Helpers,
) -> Vec<Section> {
    vec![
        sync_client(cfg),
        server(cfg, view),
        tts(cfg, h),
        helpers(h),
        backups(cfg),
        network(cfg),
        settings(cfg, language),
        self::plugins(plugins_list, patches),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag_facts::Redactor;

    fn show(s: &Section) -> String {
        let scrub = crate::debug_log::Scrubber::new(Default::default());
        Redactor {
            scrub: &scrub,
            show_host: false,
            show_titles: false,
        }
        .section(s)
    }

    #[test]
    fn the_sync_client_shows_the_shape_of_the_address_and_never_the_login() {
        let mut cfg = Config::default();
        cfg.kosync.enabled = true;
        cfg.kosync.server = "http://alice:hunter2@192.168.1.5:30260/api/koreader".into();
        cfg.kosync.user = "alice".into();
        cfg.kosync.password = "hunter2".into();
        cfg.kosync.device_name = "Alice's laptop".into();
        let t = show(&sync_client(&cfg));
        assert!(
            t.contains("- Server address: http://<credentials>@<ip:private>:30260/api/koreader"),
            "{t}"
        );
        assert!(
            t.contains("- Document matching: partial-md5") && t.contains("- Password set: yes")
        );
        for bad in ["alice", "hunter2", "192.168", "Alice's"] {
            assert!(!t.contains(bad), "{bad}: {t}");
        }
    }

    #[test]
    fn devices_seen_by_the_server_are_named_by_model_and_timed_by_class() {
        let cfg = Config::default();
        let view = ServerView {
            running: true,
            rejected: vec![(401, 3), (404, 1)],
            devices: vec![
                SeenDevice {
                    reported: "Onyx Scubapilot-IDP".into(),
                    client: "KOReader 2026.07.2".into(),
                    plugin: "0.3.1".into(),
                    protocol: 2,
                    requests: 120,
                    last_sync_ago: Some(90),
                    last_seen_ago: 30,
                },
                SeenDevice {
                    reported: "Alice's Kobo".into(),
                    client: "KOReader".into(),
                    last_sync_ago: None,
                    last_seen_ago: 400_000,
                    ..Default::default()
                },
            ],
            events: 4,
        };
        let t = show(&server(&cfg, &view));
        assert!(
            t.contains("- Device 1: Boox Leaf 2") && t.contains("- Device 1 client: 2026.07.2"),
            "{t}"
        );
        assert!(
            t.contains("- Device 1 last successful sync: under 5 minutes")
                && t.contains("- Device 2: <custom name>"),
            "{t}"
        );
        assert!(t.contains("- Device 2 last successful sync: never"));
        assert!(
            t.contains("- Requests the server refused: 401×3, 404×1"),
            "{t}"
        );
        assert!(!t.contains("Alice"));
    }

    #[test]
    fn backups_name_the_kind_and_the_shape_of_the_address_only() {
        let mut cfg = Config::default();
        cfg.health
            .backup
            .destinations
            .push(bookx_backup::Destination {
                id: "x".into(),
                name: "Alice's NAS".into(),
                kind: bookx_backup::DestKind::Webdav,
                location: "https://alice:pw@nas.example.org/remote.php/dav/files/alice/Books"
                    .into(),
                user: "alice".into(),
                encrypt: true,
                ..Default::default()
            });
        let t = show(&backups(&cfg));
        assert!(t.contains("webdav, encrypted yes"), "{t}");
        assert!(
            t.contains("https://<credentials>@<domain>/remote.php/dav/files/<segment>/books"),
            "{t}"
        );
        assert!(!t.contains("Alice") && !t.contains("nas.example") && !t.contains("pw"));
    }

    #[test]
    fn the_books_involved_show_titles_only_when_allowed() {
        let d = std::env::temp_dir().join(format!("bookx-diagb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let mut lib = crate::core::library::Library::in_memory(&d.join("lib")).unwrap();
        let p = d.join("x.md");
        std::fs::write(
            &p,
            "---\ntitle: \"The Secret Garden\"\nauthor: \"Frances Burnett\"\n---\n# A\n\nHi.\n",
        )
        .unwrap();
        lib.add_path(&p, true).unwrap();
        let id = lib.books[0].id;
        let sec = books_involved(&lib, &[id, 9999]);
        let scrub = crate::debug_log::Scrubber::new(Default::default());
        let off = Redactor {
            scrub: &scrub,
            show_host: false,
            show_titles: false,
        }
        .section(&sec);
        assert!(!off.contains("Secret") && !off.contains("Burnett"), "{off}");
        assert!(
            off.contains("Book 1 title: (not included)")
                && off.contains("Book 2 in the library: no")
        );
        let on = Redactor {
            scrub: &scrub,
            show_host: false,
            show_titles: true,
        }
        .section(&sec);
        assert!(
            on.contains("The Secret Garden") && on.contains("Frances Burnett"),
            "{on}"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn book_ids_come_from_the_trace_newest_first_without_doubles() {
        let mut o = crate::trace::op(crate::trace::Area::Sync, "zz_ids_probe");
        o.book(7);
        o.ok();
        let mut o = crate::trace::op(crate::trace::Area::Reader, "zz_ids_probe2");
        o.book(9).book(9);
        o.ok();
        let ids = book_ids(&crate::trace::snapshot());
        let (a, b) = (
            ids.iter().position(|i| *i == 9),
            ids.iter().position(|i| *i == 7),
        );
        assert!(a.is_some() && b.is_some() && a < b);
    }

    #[test]
    fn connected_devices_are_told_by_model_connection_and_state() {
        let rows = device_rows(&[
            ConnectedDevice {
                reported: "leaf2".into(),
                id: "mtp:1".into(),
                mounted: false,
                free: None,
            },
            ConnectedDevice {
                reported: "Alice's Kobo".into(),
                id: "/run/media/alice/KOBO".into(),
                mounted: true,
                free: Some((50 << 20, 8 << 30)),
            },
        ]);
        let mut sec = Section::new("Devices");
        sec.rows = rows;
        let t = show(&sec);
        for want in [
            "Connected devices: 2",
            "Device 1: Boox Leaf 2",
            "Device 1 connection: mtp",
            "Device 1 mounted: no",
            "Device 2: <custom name>",
            "Device 2 connection: usb",
            "Device 2 nearly full: yes",
        ] {
            assert!(t.contains(want), "{want} in {t}");
        }
        assert!(!t.contains("alice") && !t.contains("Alice"));
    }

    #[test]
    fn times_are_told_as_classes() {
        assert_eq!(ago_class(10), "under 5 minutes");
        assert_eq!(ago_class(4000), "under a day");
        assert_eq!(ago_class(10_000_000), "over a week");
        assert_eq!(client_version("KOReader 2026.07.2"), "2026.07.2");
    }
}
