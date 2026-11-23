//! The sentinel corpus: proof that the report holds no personal data.
//!
//! A fake user is made up whose every personal value is a *sentinel* (`SENT…`: easy to grep, impossible
//! to appear by chance). The sentinels are put into every source the report reads: the settings, the
//! library (titles, authors, folder), the log (panic messages and error strings that embed paths, URLs
//! and titles), the trace, the devices the built-in server knows, the plugins, the Flatpak info, the
//! environment. The report is built in every mode and the tests assert that no sentinel is in it, that
//! the opt-in titles and host appear only when allowed and only where intended, and, with generated
//! text of the shapes that carry personal data (paths, addresses, e-mail addresses), that the cleaners
//! do not depend on the sentinels being known.
//!
//! What cannot be caught: a name the user types in free text under "What happened" (it is shared as
//! typed, on purpose: the dialog says so), and a personal word in a message that has no recognisable
//! shape and is not one of the values the settings know. Nothing the report reads contains such a word:
//! see `docs/diagnostic-report-notes.md`, "What cannot be caught".

use crate::config::Config;
use crate::diag_gather::{SeenDevice, ServerView, World};
use crate::report::{self, Environment, Options};
use crate::trace::{Area, Entry, ErrorClass};

pub const USER: &str = "sentuser";
pub const REALNAME: &str = "Sentinel Realperson";
pub const HOSTNAME: &str = "sent-host-machine";
pub const IPV4_PUBLIC: &str = "203.0.113.77";
pub const IPV4_LAN: &str = "10.77.77.77";
pub const IPV6: &str = "2001:db8:77::77";
pub const MAC: &str = "aa:bb:cc:77:77:77";
pub const EMAIL: &str = "sent.mail77@sentinel-mail.example";
pub const SERVER_HOST: &str = "sent-sync.sentinel.example";
pub const ACCOUNT: &str = "SENTACCT77";
pub const PASSWORD: &str = "SENT-pw-7777";
pub const TOKEN: &str = "SENTtoken7777aaaaBBBBccccDDDD1234";
pub const SHARE_PW: &str = "SENT-share-pw-88";
pub const READWISE: &str = "SENT-readwise-token-9999";
pub const LIBRARY_DIR: &str = "SENT-Library-Dir";
pub const TITLE_ONE: &str = "SENT Title One";
pub const TITLE_TWO: &str = "SENT Title Two";
pub const AUTHOR_ONE: &str = "SENT Author One";
pub const AUTHOR_TWO: &str = "SENT Author Two";
pub const FILE_NAME: &str = "SENT_File_Name";
pub const DEVICE_SERIAL: &str = "SENTSERIAL77";
pub const DEVICE_NAME: &str = "SENT-DEVNAME-77";
pub const KOBO_ACCOUNT: &str = "SENT-KOBO-ACCT";
pub const SEARCH_QUERY: &str = "SENT query words";
pub const BACKUP_PLACE: &str = "SENT-Backup-Place";
pub const BACKUP_PATH: &str = "SENT-NAS";
pub const BACKUP_USER: &str = "SENTBACKUPUSER";
pub const PLUGIN_EMAIL: &str = "sent.plugin@sentinel-mail.example";
pub const SSID: &str = "SENT-WIFI-SSID";
pub const CLIPBOARD: &str = "SENT clipboard words";
pub const PROXY_HOST: &str = "sent-proxy.sentinel.example";

/// Everything that must never appear in a report in the default mode.
pub fn forbidden() -> Vec<&'static str> {
    vec![
        USER, REALNAME, HOSTNAME, IPV4_PUBLIC, IPV4_LAN, IPV6, MAC, EMAIL, "sentinel-mail", SERVER_HOST,
        "sentinel.example", ACCOUNT, PASSWORD, TOKEN, SHARE_PW, READWISE, LIBRARY_DIR, TITLE_ONE, TITLE_TWO,
        "Title One", "Title Two", AUTHOR_ONE, AUTHOR_TWO, "Author One", FILE_NAME, DEVICE_SERIAL, DEVICE_NAME,
        KOBO_ACCOUNT, SEARCH_QUERY, BACKUP_PLACE, BACKUP_PATH, BACKUP_USER, PLUGIN_EMAIL, SSID, CLIPBOARD,
        PROXY_HOST, "SENT-",
    ]
}

pub fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-pii-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub struct Fixture {
    pub dir: std::path::PathBuf,
    pub cfg: Config,
    pub lib: crate::core::library::Library,
    pub id_one: u64,
    pub id_two: u64,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn fixture() -> Fixture {
    let dir = tmp("world");
    let mut lib = crate::core::library::Library::in_memory(&dir.join(LIBRARY_DIR)).unwrap();
    let mut ids = Vec::new();
    for (t, a, f) in [(TITLE_ONE, AUTHOR_ONE, FILE_NAME), (TITLE_TWO, AUTHOR_TWO, "SENT_Other_File")] {
        let p = dir.join(format!("{f}.md"));
        std::fs::write(&p, format!("---\ntitle: \"{t}\"\nauthor: \"{a}\"\n---\n# A\n\nHello.\n")).unwrap();
        lib.add_path(&p, true).unwrap();
        ids.push(lib.books.last().unwrap().id);
    }
    let mut cfg = Config::default();
    cfg.kosync.enabled = true;
    cfg.kosync.server = format!("http://{ACCOUNT}:{PASSWORD}@{SERVER_HOST}:30260/api/koreader/SENTsegment?token={TOKEN}");
    cfg.kosync.user = ACCOUNT.into();
    cfg.kosync.password = PASSWORD.into();
    cfg.kosync.device_name = DEVICE_NAME.into();
    cfg.share.password = SHARE_PW.into();
    cfg.readwise_token = READWISE.into();
    cfg.opds_catalogs.push(crate::opds_browse::Catalog {
        id: "c1".into(),
        name: format!("{REALNAME} library"),
        url: format!("https://{SERVER_HOST}/opds/{ACCOUNT}"),
        user: ACCOUNT.into(),
        password: PASSWORD.into(),
    });
    cfg.health.backup.destinations.push(bookx_backup::Destination {
        id: "b1".into(),
        name: BACKUP_PLACE.into(),
        kind: bookx_backup::DestKind::Webdav,
        location: format!("https://{SERVER_HOST}/dav/{BACKUP_USER}/backups"),
        user: BACKUP_USER.into(),
        password: PASSWORD.into(),
        ..Default::default()
    });
    cfg.health.backup.destinations.push(bookx_backup::Destination {
        id: "b2".into(),
        name: BACKUP_PLACE.into(),
        kind: bookx_backup::DestKind::Folder,
        location: format!("/mnt/{BACKUP_PATH}/backups"),
        ..Default::default()
    });
    Fixture { dir, cfg, lib, id_one: ids[0], id_two: ids[1] }
}

fn entry(area: Area, name: &str, ok: bool, class: Option<ErrorClass>, http: Option<u16>, fields: &[(&str, &str)], msg: Option<&str>) -> Entry {
    Entry {
        time: 1_763_900_000,
        area,
        name: name.into(),
        ok,
        class,
        http,
        ms: Some(42),
        fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        msg: msg.map(str::to_string),
        repeats: 1,
    }
}

/// The log of the fake user: warnings, errors and a panic that carry the sentinels in the shapes real
/// messages have.
pub fn dirty_log() -> Vec<String> {
    let home = format!("/home/{USER}");
    vec![
        format!("2026-11-23T10:00:00Z  INFO bookx_crumb: start Bookx 0.3.0-beta.3 (beta), Flatpak"),
        format!("2026-11-23T10:00:01Z ERROR bookx: cannot open library {home}/{LIBRARY_DIR}: permission denied"),
        format!("2026-11-23T10:00:02Z  WARN bookx::import: import failed: {home}/{LIBRARY_DIR}/{FILE_NAME}.epub: not a zip archive"),
        format!("2026-11-23T10:00:03Z  WARN bookx::kosync: talking to http://{ACCOUNT}:{PASSWORD}@{SERVER_HOST}:30260/api/koreader?token={TOKEN}: connection refused"),
        format!("2026-11-23T10:00:04Z  WARN bookx: no such file: '{TITLE_ONE}.pdf'"),
        format!("2026-11-23T10:00:05Z  WARN bookx::mail: login for {EMAIL} failed from {IPV4_PUBLIC} and {IPV4_LAN} ({IPV6}) mac {MAC}"),
        format!("2026-11-23T10:00:06Z ERROR panic: called `Result::unwrap()` on an `Err` value: Os {{ code: 2 }} at {home}/{LIBRARY_DIR}/{TITLE_TWO}.epub:12:5"),
        format!("2026-11-23T10:00:07Z  WARN bookx::search: search for \"{SEARCH_QUERY}\" in {TITLE_ONE} by {AUTHOR_ONE} gave no result"),
        format!("2026-11-23T10:00:08Z  WARN bookx::device: copy to /run/media/{USER}/KOBOeReader/{FILE_NAME}.kepub.epub failed (serial {DEVICE_SERIAL}, account {KOBO_ACCOUNT})"),
        format!("2026-11-23T10:00:09Z  WARN bookx::backup: place {BACKUP_PLACE} at /mnt/{BACKUP_PATH}/backups failed: no space left on device"),
        format!("2026-11-23T10:00:10Z  WARN bookx::plugin: plugin {PLUGIN_EMAIL} could not be loaded"),
        format!("2026-11-23T10:00:11Z  WARN bookx: host {HOSTNAME} proxy http://{PROXY_HOST}:3128 clipboard \"{CLIPBOARD}\" {REALNAME}"),
    ]
}

/// The fake world: the settings and the library of the fake user, a trace and a server that know them.
pub fn world<'a>(fx: &'a Fixture, entry_area: Option<&str>) -> World<'a> {
    let home = format!("/home/{USER}");
    let trace = vec![
        entry(Area::Sync, "login", true, None, None, &[], None),
        entry(
            Area::Sync,
            "push",
            false,
            Some(ErrorClass::Status),
            Some(404),
            &[("fmt", "pdf"), ("size", "lt10mb"), ("doc", "0123456789abcdef0123456789abcdef"), ("method", "partial-md5"), ("book", &fx.id_one.to_string()), ("local", "0.373"), ("remote", "none")],
            Some(&format!("the server answered 404 talking to http://{SERVER_HOST}:30260/api/koreader/users/{ACCOUNT}")),
        ),
        entry(Area::Import, "add", false, Some(ErrorClass::Corrupt), None, &[("fmt", "epub")], Some(&format!("{home}/{LIBRARY_DIR}/{FILE_NAME}.epub is not a zip archive"))),
        entry(Area::Devices, "copy", false, Some(ErrorClass::NotMounted), None, &[("device", "Boox_Leaf_2")], Some(&format!("{DEVICE_NAME} is not connected any more ({KOBO_ACCOUNT})"))),
        entry(Area::Backup, "run", false, Some(ErrorClass::DiskFull), None, &[], Some(&format!("no space left on device writing /mnt/{BACKUP_PATH}/backups/{BACKUP_PLACE}.bookxbackup"))),
        entry(Area::Search, "query", false, Some(ErrorClass::Other), None, &[], Some(&format!("bad query \"{SEARCH_QUERY}\""))),
    ];
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("https_proxy".to_string(), format!("http://{ACCOUNT}:{PASSWORD}@{PROXY_HOST}:3128"));
    vars.insert("USER".to_string(), USER.to_string());
    vars.insert("HOSTNAME".to_string(), HOSTNAME.to_string());
    vars.insert("WIFI".to_string(), SSID.to_string());
    World {
        env: Environment {
            version: "0.3.0-beta.3".into(),
            channel: "beta".into(),
            commit: "abc123".into(),
            profile: "release".into(),
            packaging: "Flatpak".into(),
            os: "GNOME 51 (Flatpak runtime)".into(),
            kernel: "6.17.0".into(),
            desktop: "GNOME".into(),
            session: "wayland".into(),
            gtk: "4.24.1".into(),
            adw: "1.10.0".into(),
            language: "en".into(),
            locale: "en_US.UTF-8".into(),
            scale: 2,
            startup_ms: Some(800),
        },
        cfg: &fx.cfg,
        lib: &fx.lib,
        indexed: Some((2, 2)),
        plugins: vec![bookx_plugin::PluginStatus {
            id: PLUGIN_EMAIL.into(),
            name: format!("{REALNAME}'s plugin"),
            version: "1.0.0".into(),
            kind: None,
            description: format!("by {EMAIL}"),
            dir: format!("{home}/.var/app/plugins/x").into(),
            permissions: vec![],
            actions: vec![],
            enabled: true,
            loaded: false,
            error: Some(format!("cannot read {home}/.local/share/plugins/x.wasm for {ACCOUNT}")),
        }],
        patches: 1,
        keyring: "system keyring".into(),
        server: ServerView {
            running: true,
            rejected: vec![(401, 2)],
            events: 3,
            devices: vec![
                SeenDevice { reported: DEVICE_NAME.into(), client: "KOReader 2026.07.2".into(), plugin: "0.3.1".into(), protocol: 2, requests: 5, last_sync_ago: Some(60), last_seen_ago: 10 },
                SeenDevice { reported: format!("{REALNAME}'s Kobo"), client: "KOReader".into(), ..Default::default() },
            ],
        },
        jobs: vec![report::JobLine { kind: "Backup".into(), status: format!("failed: cannot write /mnt/{BACKUP_PATH}/x for {ACCOUNT}") }],
        log: dirty_log(),
        trace: trace.clone(),
        previous: trace.into_iter().take(2).collect(),
        crash: Some(format!(
            "Bookx 0.3.0-beta.3 (beta) closed unexpectedly at 2026-11-23 10:00:00 UTC\nthread: main\nmessage: cannot open {home}/{LIBRARY_DIR}/{TITLE_ONE}.epub as {ACCOUNT}\nat: src/x.rs:1:1\n\nbacktrace:\n   0: bookx::x {home}/src/x.rs:1\n"
        )),
        flatpak_info: Some(format!(
            "[Application]\nruntime=runtime/org.gnome.Platform/x86_64/51\n[Instance]\nflatpak-version=1.16.1\ninstance-path={home}/.var/app/com.adhadse.Bookx\n[Context]\nshared=network;\nsockets=wayland;pulseaudio;\nfilesystems=/home/{USER}/{LIBRARY_DIR}:rw;!/mnt/{BACKUP_PATH};xdg-download/{TITLE_ONE};\n"
        )),
        backend: "wayland".into(),
        renderer: "ngl".into(),
        vars,
        extra: vec![(
            Area::Tts,
            vec![("Voice".to_string(), crate::diag_facts::Fact::text(&format!("{REALNAME} voice in {home}/models")))],
        )],
        entry: entry_area.map(str::to_string),
        probe: None,
    }
}

pub fn scrubber(fx: &Fixture) -> crate::debug_log::Scrubber {
    let mut rules = crate::diag_gather::scrub_rules(
        &fx.cfg,
        &fx.lib,
        vec![DEVICE_NAME.into(), format!("{REALNAME}'s Kobo"), KOBO_ACCOUNT.into(), REALNAME.into(), SEARCH_QUERY.into(), BACKUP_PLACE.into(), BACKUP_PATH.into(), CLIPBOARD.into(), PROXY_HOST.into()],
        HOSTNAME,
    );
    rules.home = format!("/home/{USER}");
    rules.user = USER.into();
    crate::debug_log::Scrubber::new(rules)
}

pub fn render(fx: &Fixture, opts: Options, entry: Option<&str>) -> String {
    let w = world(fx, entry);
    let input = crate::diag_gather::input(&w);
    report::build(&input, opts, "", &scrubber(fx))
}

fn leaks(r: &str, allow: &[&str]) -> Vec<String> {
    forbidden()
        .into_iter()
        .filter(|b| !allow.contains(b))
        .filter(|b| r.contains(b))
        .map(|b| b.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_sentinel_is_in_the_report_in_the_default_mode() {
        let fx = fixture();
        let r = render(&fx, Options::default(), None);
        assert_eq!(leaks(&r, &[]), Vec::<String>::new(), "\n{r}");
        // the report is not empty because everything was dropped: the facts are there
        for want in [
            "### Summary and suspected area",
            "### Recent operations",
            "sync/push FAIL http=404 class=status",
            "doc=0123456789abcdef0123456789abcdef",
            "Server address: http://<credentials>@<domain>:30260/api/koreader/<segment>?<query>",
            "Book 1 title: (not included)",
            "Device 1: <custom name>",
        ] {
            assert!(r.contains(want), "{want} missing in\n{r}");
        }
    }

    #[test]
    fn no_sentinel_is_in_the_report_with_each_switch_off_or_on_without_the_opt_ins() {
        let fx = fixture();
        for (log, lib) in [(false, false), (true, false), (false, true), (true, true)] {
            let r = render(&fx, Options { include_log: log, include_library: lib, ..Options::default() }, Some("Sync (KOSync client)"));
            assert_eq!(leaks(&r, &[]), Vec::<String>::new(), "log={log} lib={lib}\n{r}");
        }
    }

    #[test]
    fn titles_appear_only_when_allowed_and_only_for_the_books_involved() {
        let fx = fixture();
        let on = render(&fx, Options { include_titles: true, ..Options::default() }, None);
        assert!(on.contains(TITLE_ONE) && on.contains(AUTHOR_ONE), "{on}");
        // the second book was not involved in anything
        assert!(!on.contains(TITLE_TWO) && !on.contains(AUTHOR_TWO), "{on}");
        // and everything else stays hidden
        assert_eq!(leaks(&on, &[TITLE_ONE, "Title One", AUTHOR_ONE, "Author One"]), Vec::<String>::new(), "{on}");
        // only in the section of the books involved
        let hits: Vec<&str> = on.lines().filter(|l| l.contains(TITLE_ONE)).collect();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].starts_with("- Book 1 title: "));
    }

    #[test]
    fn the_host_appears_only_when_allowed_and_only_on_address_lines() {
        let fx = fixture();
        let on = render(&fx, Options { include_host: true, ..Options::default() }, None);
        let lines: Vec<&str> = on.lines().filter(|l| l.contains(SERVER_HOST)).collect();
        assert!(!lines.is_empty(), "the host is shown when allowed:\n{on}");
        for l in &lines {
            assert!(l.contains("address: ") || l.starts_with("- Server address"), "{l}");
        }
        assert_eq!(leaks(&on, &[SERVER_HOST, "sentinel.example"]), Vec::<String>::new(), "{on}");
        // credentials, query and tokens are never shown, whatever the switch
        assert!(!on.contains(ACCOUNT) && !on.contains(PASSWORD) && !on.contains(TOKEN));
    }

    #[test]
    fn both_opt_ins_together_still_hide_everything_else() {
        let fx = fixture();
        let r = render(&fx, Options { include_titles: true, include_host: true, ..Options::default() }, Some("Sync (KOSync client)"));
        assert_eq!(leaks(&r, &[TITLE_ONE, "Title One", AUTHOR_ONE, "Author One", SERVER_HOST, "sentinel.example"]), Vec::<String>::new(), "{r}");
    }

    #[test]
    fn realistic_library_error_messages_lose_their_paths_addresses_and_titles() {
        let fx = fixture();
        let s = scrubber(&fx);
        let red = crate::diag_facts::Redactor { scrub: &s, show_host: false, show_titles: false };
        for (msg, forbidden) in [
            (format!("cannot open /home/{USER}/Books/Foo/Foo.epub"), vec![USER, "Foo"]),
            ("talking to http://10.0.0.7:30260/api/koreader".to_string(), vec!["10.0.0.7"]),
            (format!("no such file: '{TITLE_ONE}.pdf'"), vec!["Title One", ".pdf'"]),
            (format!("SQLite error: unable to open database file {}/bookx-library.db", fx.lib.root.display()), vec![LIBRARY_DIR]),
            (format!("No such file or directory (os error 2) at /run/media/{USER}/KOBOeReader/documents/{FILE_NAME}.epub"), vec![USER, FILE_NAME, "KOBOeReader"]),
            (format!("HTTP status client error (404 Not Found) for url (https://{SERVER_HOST}/dav/{BACKUP_USER}/x?token={TOKEN})"), vec![SERVER_HOST, BACKUP_USER, TOKEN]),
            (format!("failed to read ~/Documents/{TITLE_TWO}/{FILE_NAME}.cbz"), vec!["Title Two", FILE_NAME, "Documents"]),
            (format!("invalid zip: central directory of \"{TITLE_ONE} (2nd edition).epub\" is damaged"), vec!["Title One", "2nd edition"]),
        ] {
            let c = red.text(&msg);
            for f in &forbidden {
                assert!(!c.contains(f), "{f} left in `{c}` from `{msg}`");
            }
        }
    }

    /// A small deterministic generator, so the property tests need no extra crate.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn word(&mut self) -> String {
            format!("zq{:08x}", self.next() as u32)
        }
        fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
            &v[(self.next() as usize) % v.len()]
        }
    }

    #[test]
    fn random_text_with_the_shapes_of_personal_data_is_cleaned_without_knowing_the_values() {
        let s = crate::debug_log::Scrubber::new(Default::default());
        let red = crate::diag_facts::Redactor { scrub: &s, show_host: false, show_titles: false };
        let mut g = Lcg(0x5eed);
        for _ in 0..3000 {
            let (a, b, c) = (g.word(), g.word(), g.word());
            let ext = *g.pick(&["epub", "pdf", "mobi", "cbz", "fb2", "kepub.epub"]);
            let port = 1000 + g.next() % 60000;
            let (text, secret) = match g.next() % 8 {
                0 => (format!("cannot open /home/{a}/{b}/{c}.{ext}"), vec![a.clone(), b.clone(), c.clone()]),
                1 => (format!("talking to http://{a}.{b}:{port}/{c}"), vec![a.clone(), b.clone(), c.clone()]),
                2 => (format!("https://{a}:{b}@{c}.example.org/x"), vec![a.clone(), b.clone(), c.clone()]),
                3 => (format!("mail {a}.{b}@{c}.org failed"), vec![a.clone(), b.clone(), c.clone()]),
                4 => (format!("no such file: '{a} {b}.{ext}'"), vec![a.clone(), b.clone()]),
                5 => (format!("copy to /run/media/{a}/{b}/{c}.{ext} failed"), vec![a.clone(), b.clone(), c.clone()]),
                6 => (format!("open ~/{a}/{b} {c}.{ext} now"), vec![a.clone(), b.clone(), c.clone()]),
                _ => (format!("get http://10.{}.{}.{}:{port}/{a}/{b}", g.next() % 256, g.next() % 256, g.next() % 256), vec![a.clone(), b.clone()]),
            };
            let out = red.text(&text);
            for w in secret {
                assert!(!out.contains(&w), "{w} left in `{out}` from `{text}`");
            }
        }
    }

    #[test]
    fn random_messages_in_the_trace_never_reach_the_report_raw() {
        let fx = fixture();
        let mut g = Lcg(42);
        for _ in 0..25 {
            let (a, b) = (g.word(), g.word());
            let mut w = world(&fx, None);
            w.trace.push(entry(Area::Import, "add", false, Some(ErrorClass::Io), None, &[], Some(&format!("cannot read /home/{a}/{b}.epub from http://{b}.example.org:8080/{a}"))));
            let input = crate::diag_gather::input(&w);
            let r = report::build(&input, Options::default(), "", &scrubber(&fx));
            assert!(!r.contains(&a) && !r.contains(&b), "{a} {b}\n{r}");
        }
    }

    #[test]
    fn the_user_text_is_the_only_free_text_and_it_is_kept_as_typed() {
        let fx = fixture();
        let w = world(&fx, None);
        let input = crate::diag_gather::input(&w);
        let r = report::build(&input, Options::default(), "I use SENT-typed-by-me", &scrubber(&fx));
        assert!(r.ends_with("### What happened\n\nI use SENT-typed-by-me\n"));
        let head = r.split("### What happened").next().unwrap();
        assert!(!head.contains("SENT-typed-by-me"));
    }

    #[test]
    fn every_fact_goes_through_the_redactor_not_through_a_raw_push() {
        // `Fact` cannot be made from a string (its kind is private), and the report builder reads a
        // section only through `Redactor::section`: nothing in `build` touches the rows directly.
        let src = include_str!("report.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains(".rows"), "report.rs reads section rows directly");
        assert!(code.contains("red.section(sec)"));
        // the modules that make facts do not build report text themselves
        for (name, text) in [
            ("diag_gather.rs", include_str!("diag_gather.rs")),
            ("diag_system.rs", include_str!("diag_system.rs")),
        ] {
            let code = text.split("#[cfg(test)]").next().unwrap();
            assert!(!code.contains("push_str"), "{name} builds text by hand");
        }
    }

    #[test]
    fn the_dialog_text_and_the_report_agree_on_what_is_hidden() {
        let fx = fixture();
        let r = render(&fx, Options::default(), None);
        assert!(r.contains("(not included)") && r.contains("<host>") || r.contains("<domain>"));
    }

    #[test]
    fn the_log_file_text_is_cleaned_like_the_report() {
        let fx = fixture();
        let s = scrubber(&fx);
        for l in dirty_log() {
            let c = crate::applog::clean_line(&l, &s);
            let c = crate::diag_addr::clean_paths(&c);
            assert_eq!(leaks(&c, &[]), Vec::<String>::new(), "{l}\n-> {c}");
        }
    }
}
