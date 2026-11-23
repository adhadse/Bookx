//! Validation by replay: can a maintainer find the cause from the report alone?
//!
//! Each [`Case`] is a failure the app has had or plausibly will have, written as the state, trace and
//! log the app would hold when it happened. The report is built the way the app builds it
//! (`diag_gather::input` and `report::build`) and the test asserts two things: the *Summary and
//! suspected area* names the right area, and the report holds the facts a maintainer needs for that case
//! (each a pattern, listed per case). The measured result is the coverage table in
//! `docs/diagnostic-report-notes.md`; a test keeps the table and this list the same.
//!
//! The verdicts are honest: `Yes` means the report alone is enough, `Partly` that the report narrows the
//! cause but a question remains (a visual defect leaves no error), `No` that the app cannot make a
//! report at all (it does not start). The cases are written by the people who wrote the recorder, so
//! the coverage is a lower bound on how well the recorder was fitted to its own failures, not a measure
//! of the unknown ones; the table says so.

use crate::config::Config;
use crate::diag_gather::{Helpers, SeenDevice, ServerView, World};
use crate::diag_pii::entry as ev;
use crate::diag_system::{DbProbe, LibraryProbe};
use crate::report::{self, Environment, Options};
use crate::trace::{Area, Entry, ErrorClass as C};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Yes,
    Partly,
    No,
}

/// What a case sets up; everything else is a quiet, healthy app.
pub struct Sc {
    pub cfg: Config,
    pub trace: Vec<Entry>,
    pub log: Vec<String>,
    pub crash: Option<String>,
    pub flatpak: Option<String>,
    pub server: ServerView,
    pub probe: LibraryProbe,
    pub env: Environment,
    pub keyring: String,
    pub helpers: Helpers,
    pub book: u64,
    pub indexed: Option<(usize, usize)>,
    pub extra: Vec<(Area, Vec<(String, crate::diag_facts::Fact)>)>,
}

pub struct Case {
    pub name: &'static str,
    pub verdict: Verdict,
    pub why: &'static str,
    /// The area the summary must name (empty when it cannot).
    pub area: &'static str,
    /// Patterns that must match the report.
    pub must: &'static [&'static str],
    pub setup: fn(&mut Sc),
}

fn healthy_probe() -> LibraryProbe {
    LibraryProbe {
        mountinfo: "29 1 8:2 / / rw,relatime - ext4 /dev/sda2 rw\n".into(),
        canon: "/home/u/Books".into(),
        free: Some((200 << 30, 500 << 30)),
        sync_client: None,
        db: Some(DbProbe { size: 4 << 20, copies: 0, problems: Some(0) }),
        read_only: false,
    }
}

fn env() -> Environment {
    Environment {
        version: "0.3.0-beta.3".into(),
        channel: "beta".into(),
        commit: "abc".into(),
        profile: "release".into(),
        packaging: "Flatpak".into(),
        os: "GNOME 51 (Flatpak runtime)".into(),
        kernel: "6.17".into(),
        desktop: "GNOME".into(),
        session: "wayland".into(),
        gtk: "4.24.1".into(),
        adw: "1.10.0".into(),
        language: "en".into(),
        locale: "en_US.UTF-8".into(),
        scale: 2,
        startup_ms: Some(700),
    }
}

const FLATPAK_NET: &str = "[Application]\nruntime=runtime/org.gnome.Platform/x86_64/51\n[Instance]\nflatpak-version=1.16.1\n[Context]\nshared=network;ipc;\nsockets=wayland;pulseaudio;\ndevices=dri;\nfilesystems=xdg-download;\n";
const FLATPAK_NO_NET: &str = "[Application]\nruntime=runtime/org.gnome.Platform/x86_64/51\n[Instance]\nflatpak-version=1.16.1\n[Context]\nshared=ipc;\nsockets=wayland;pulseaudio;\ndevices=dri;\nfilesystems=xdg-download;\n";

fn base() -> Sc {
    let mut cfg = Config::default();
    cfg.kosync.enabled = false;
    Sc {
        cfg,
        trace: vec![ev(Area::Database, "open_library", true, None, None, &[("books", "10-99")], None)],
        log: vec!["2026-11-23T10:00:00Z  INFO bookx_crumb: start Bookx 0.3.0-beta.3 (beta), Flatpak".into()],
        crash: None,
        flatpak: Some(FLATPAK_NET.into()),
        server: ServerView::default(),
        probe: healthy_probe(),
        env: env(),
        keyring: "system keyring".into(),
        helpers: Helpers { rar: true, djvu: true, espeak: true, audio_player: true },
        book: 0,
        indexed: Some((1, 1)),
        extra: vec![],
    }
}

fn kosync_cfg(c: &mut Config, server: &str, by_filename: bool) {
    c.kosync.enabled = true;
    c.kosync.server = server.into();
    c.kosync.user = "someone".into();
    c.kosync.password = "pw".into();
    c.kosync.logged_in = true;
    c.kosync.by_filename = by_filename;
}

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| format!("2026-11-23T10:05:00Z {s}")).collect()
}

const DOC: &str = "0123456789abcdef0123456789abcdef";

pub fn cases() -> Vec<Case> {
    use Verdict::*;
    vec![
        Case {
            name: "kosync-push-404-pdf-grimmory",
            verdict: Yes,
            why: "issue #49: the address shape, the matching method, the book's format and hash, and the HTTP status are all in the summary and the sync section",
            area: "Sync (KOSync client)",
            must: &[
                r"sync/push FAIL[^\n]*http=404",
                r"Server address: http://<ip:private>:30260/api/koreader",
                r"Document matching: partial-md5",
                r"sync/push FAIL[^\n]*fmt=pdf",
                r"doc=[0-9a-f]{32}",
                r"Book 1 format: pdf",
                r"Logged in: yes",
            ],
            setup: |s| {
                kosync_cfg(&mut s.cfg, "http://192.168.1.5:30260/api/koreader", false);
                s.trace.push(ev(Area::Sync, "login", true, None, None, &[], None));
                s.trace.push(ev(Area::Sync, "pull", true, None, None, &[("fmt", "pdf"), ("size", "lt10mb"), ("method", "partial-md5"), ("book", &s.book.to_string()), ("local", "0.373"), ("remote", "none"), ("doc", DOC)], None));
                s.trace.push(ev(Area::Sync, "push", false, Some(C::Status), Some(404), &[("fmt", "pdf"), ("size", "lt10mb"), ("method", "partial-md5"), ("book", &s.book.to_string()), ("local", "0.373"), ("doc", DOC)], Some("the server answered 404")));
            },
        },
        Case {
            name: "boox-leaf2-expects-books-to-sync",
            verdict: Yes,
            why: "issue #39: nothing failed; the report names the device as a Boox Leaf 2, shows the plugin and that it never completed a sync, and says what the server does and does not move",
            area: "Built-in server",
            must: &[r"Device 1: Boox Leaf 2", r"Device 1 plugin: 0\.3\.1", r"Device 1 last successful sync: never", r"Running: yes", r"Reading positions \(KOSync\): on"],
            setup: |s| {
                s.cfg.share.kosync = true;
                s.server = ServerView {
                    running: true,
                    devices: vec![SeenDevice { reported: "Onyx Scubapilot-IDP".into(), client: "KOReader 2026.07.2".into(), plugin: "0.3.1".into(), protocol: 2, requests: 12, last_sync_ago: None, last_seen_ago: 30 }],
                    ..Default::default()
                };
            },
        },
        Case {
            name: "package-built-for-another-release",
            verdict: No,
            why: "the program does not start (glibc symbol error): the app cannot build a report; build-aux/triage-report.py recognises the pasted terminal output instead",
            area: "",
            must: &[],
            setup: |_| {},
        },
        Case {
            name: "beta-notice-takes-focus",
            verdict: Partly,
            why: "the report shows that a notice banner appeared and what the reader did right after; whether a key reached the page stays a question",
            area: "Display and interface",
            must: &[r"ui/notice_shown[^\n]*kind=beta", r"Display backend: wayland"],
            setup: |s| {
                s.trace.push(ev(Area::Ui, "notice_shown", true, None, None, &[("kind", "beta")], None));
                s.trace.push(ev(Area::Reader, "load_chapter", true, None, None, &[], None));
            },
        },
        Case {
            name: "highlight-not-painted-after-relayout",
            verdict: Yes,
            why: "the reader records how many highlights are stored and how many belong to the chapter shown, next to the relayout and the banner",
            area: "Reader",
            must: &[r"reader/paint_highlights[^\n]*stored=3[^\n]*in_chapter=0", r"reader/load_chapter", r"ui/notice_shown"],
            setup: |s| {
                s.trace.push(ev(Area::Ui, "notice_shown", true, None, None, &[("kind", "beta")], None));
                s.trace.push(ev(Area::Reader, "load_chapter", true, None, None, &[], None));
                s.trace.push(ev(Area::Reader, "paint_highlights", true, None, None, &[("stored", "3"), ("in_chapter", "0")], None));
            },
        },
        Case {
            name: "database-locked-two-writers",
            verdict: Yes,
            why: "the open fails with class locked and the message names a second Bookx; the sync client and the file system are listed",
            area: "Library database",
            must: &[r"database/open_library FAIL[^\n]*class=locked", r"already open in", r"Synced by: none", r"File system: ext4"],
            setup: |s| {
                s.trace.push(ev(Area::Database, "open_library", false, Some(C::Locked), None, &[], Some("This library is already open in Bookx (process 4242 on a-host, since 2026-11-23 09:00 UTC). Close it there first")));
                s.log.extend(lines(&["ERROR bookx: cannot open library /home/u/Books: This library is already open in Bookx (process 4242)"]));
            },
        },
        Case {
            name: "catalogue-schema-newer-than-supported",
            verdict: Yes,
            why: "the error line, the supported schema and the copies from before an update are in the report",
            area: "Library database",
            must: &[r"newer than supported version", r"Catalogue schema: \d+", r"Pre-update copies of the catalogue: 2"],
            setup: |s| {
                s.probe.db = Some(DbProbe { size: 4 << 20, copies: 2, problems: Some(0) });
                s.trace.push(ev(Area::Database, "open_library", false, Some(C::Other), None, &[], Some("database schema version 12 is newer than supported version 9")));
                s.log.extend(lines(&["ERROR bookx: cannot open library /home/u/Books: database schema version 12 is newer than supported version 9"]));
            },
        },
        Case {
            name: "webdav-backup-401",
            verdict: Yes,
            why: "the backup job fails with class unauthorized and the place is told by kind, encryption and address shape",
            area: "Backups",
            must: &[r"backup/job FAIL[^\n]*class=unauthorized", r"Place 1: webdav, encrypted yes", r"Place 1 address: https://<domain>/remote\.php/dav/files/<segment>"],
            setup: |s| {
                s.cfg.health.backup.destinations.push(bookx_backup::Destination { id: "w".into(), name: "x".into(), kind: bookx_backup::DestKind::Webdav, location: "https://cloud.example.org/remote.php/dav/files/someone".into(), encrypt: true, ..Default::default() });
                s.trace.push(ev(Area::Backup, "job", false, Some(C::Unauthorized), Some(401), &[("kind", "backup")], Some("the server answered 401")));
            },
        },
        Case {
            name: "webdav-backup-tls-error",
            verdict: Yes,
            why: "class tls on the backup job, the scheme and the kind of host",
            area: "Backups",
            must: &[r"backup/job FAIL[^\n]*class=tls", r"Place 1 address: https://<domain>"],
            setup: |s| {
                s.cfg.health.backup.destinations.push(bookx_backup::Destination { id: "w".into(), name: "x".into(), kind: bookx_backup::DestKind::Webdav, location: "https://nas.example.org:5006/backups".into(), ..Default::default() });
                s.trace.push(ev(Area::Backup, "job", false, Some(C::Tls), None, &[("kind", "backup")], Some("invalid peer certificate: UnknownIssuer")));
            },
        },
        Case {
            name: "webdav-backup-timeout",
            verdict: Yes,
            why: "class timeout with the duration of the job",
            area: "Backups",
            must: &[r"backup/job FAIL[^\n]*class=timeout", r"Place 1: webdav"],
            setup: |s| {
                s.cfg.health.backup.destinations.push(bookx_backup::Destination { id: "w".into(), name: "x".into(), kind: bookx_backup::DestKind::Webdav, location: "https://nas.example.org/dav".into(), ..Default::default() });
                let mut e = ev(Area::Backup, "job", false, Some(C::Timeout), None, &[("kind", "backup")], Some("operation timed out"));
                e.ms = Some(30_000);
                s.trace.push(e);
            },
        },
        Case {
            name: "mtp-boox-not-mounted",
            verdict: Yes,
            why: "the copy fails with class not_mounted, the device model, the backend and the sandbox device permissions",
            area: "Devices",
            must: &[r"devices/copy FAIL[^\n]*class=not_mounted", r"device=Boox_Leaf_2", r"backend=mtp", r"Devices: dri"],
            setup: |s| {
                s.trace.push(ev(Area::Devices, "copy", false, Some(C::NotMounted), None, &[("device", "Boox_Leaf_2"), ("backend", "mtp"), ("fmt", "epub"), ("size", "lt1mb")], Some("the device is not mounted any more")));
            },
        },
        Case {
            name: "kobo-database-missing",
            verdict: Yes,
            why: "the device name is told as a model, the message says the Kobo has no library database",
            area: "Devices",
            must: &[r"device=Kobo_Clara_HD|device=Kobo", r"no library database yet", r"backend=usb"],
            setup: |s| {
                s.trace.push(ev(Area::Devices, "copy", false, Some(C::Other), None, &[("device", "Kobo_Clara_HD"), ("backend", "usb")], Some("this Kobo has no library database yet")));
            },
        },
        Case {
            name: "kepub-conversion-fails",
            verdict: Yes,
            why: "the conversion step, its target, the input size class and the error class",
            area: "Conversion",
            must: &[r"convert/epub_convert FAIL[^\n]*class=corrupt", r"to=kepub", r"fmt=epub", r"size=lt10mb"],
            setup: |s| {
                s.trace.push(ev(Area::Convert, "epub_convert", false, Some(C::Corrupt), None, &[("fmt", "epub"), ("to", "kepub"), ("size", "lt10mb")], Some("converting `x.epub` to KEPUB: the file is not a zip archive")));
            },
        },
        Case {
            name: "cbr-without-unrar",
            verdict: Yes,
            why: "the import fails for a cbr, and the helper line says neither unrar nor 7-Zip is there",
            area: "Conversion",
            must: &[r"import/prepare FAIL[^\n]*fmt=cbr", r"RAR helper \(CBR\): unrar or 7-Zip: no"],
            setup: |s| {
                s.helpers.rar = false;
                s.trace.push(ev(Area::Import, "prepare", false, Some(C::Missing), None, &[("fmt", "cbr"), ("size", "lt100mb")], Some("RAR comic archives (CBR) need the `unrar` or `7z` program; install it, or convert the file to CBZ")));
            },
        },
        Case {
            name: "djvu-without-djvulibre",
            verdict: Yes,
            why: "the import fails for a djvu and the DjVuLibre line says it is missing",
            area: "Conversion",
            must: &[r"import/prepare FAIL[^\n]*fmt=djvu", r"DjVuLibre \(DJVU\): no"],
            setup: |s| {
                s.helpers.djvu = false;
                s.trace.push(ev(Area::Import, "prepare", false, Some(C::Missing), None, &[("fmt", "djvu")], Some("DjVu books need DjVuLibre (the `ddjvu` and `djvused` programs); install it and try again")));
            },
        },
        Case {
            name: "pdf-without-text-layer-search",
            verdict: Partly,
            why: "search and read aloud on a scan fail with a generic error; the report shows the format and the failing step, but not that the PDF is a scan",
            area: "Search",
            must: &[r"search/extract_text FAIL", r"fmt=pdf"],
            setup: |s| {
                s.trace.push(ev(Area::Search, "extract_text", false, Some(C::Other), None, &[("fmt", "pdf")], Some("the book has no text to index")));
            },
        },
        Case {
            name: "tts-voice-missing",
            verdict: Yes,
            why: "open_engine fails with class missing, the engine and the helper lines",
            area: "Read aloud",
            must: &[r"tts/open_engine FAIL[^\n]*class=missing", r"Engine: piper", r"espeak-ng found: yes"],
            setup: |s| {
                s.cfg.tts.engine = "piper".into();
                s.trace.push(ev(Area::Tts, "open_engine", false, Some(C::Missing), None, &[("lang", "de")], Some("no Piper voice is downloaded")));
            },
        },
        Case {
            name: "tts-no-audio-player",
            verdict: Yes,
            why: "the helper line says no audio player is installed and the sandbox sockets are listed",
            area: "Read aloud",
            must: &[r"Audio player \(pacat, pw-cat or aplay\) found: no", r"Sockets: wayland, pulseaudio"],
            setup: |s| {
                s.helpers.audio_player = false;
            },
        },
        Case {
            name: "tts-model-hash-mismatch",
            verdict: Yes,
            why: "the download fails with class hash_mismatch and the job duration",
            area: "Read aloud",
            must: &[r"tts/download_packs FAIL[^\n]*class=hash_mismatch"],
            setup: |s| {
                s.trace.push(ev(Area::Tts, "download_packs", false, Some(C::HashMismatch), None, &[], Some("the checksum of https://<domain>/voice.onnx does not match (got aa, expected bb): the file was damaged or changed; it was deleted")));
            },
        },
        Case {
            name: "plugin-fails-to-load",
            verdict: Yes,
            why: "the plugin id and version, the load failure and the warning in the log",
            area: "Plugins and patches",
            must: &[r"plugins/load FAIL", r"Plugin 1: acme\.trim 1\.0\.0", r"could not be loaded"],
            setup: |s| {
                s.trace.push(ev(Area::Plugins, "load", false, Some(C::Other), None, &[], Some("the module traps: unreachable")));
                s.log.extend(lines(&["WARN bookx: plugin acme.trim could not be loaded: the module traps"]));
                s.extra.push((Area::Plugins, vec![]));
            },
        },
        Case {
            name: "mcp-port-in-use",
            verdict: Yes,
            why: "the start fails with class port_in_use",
            area: "AI assistants (MCP)",
            must: &[r"mcp/start FAIL[^\n]*class=port_in_use"],
            setup: |s| {
                s.trace.push(ev(Area::Mcp, "start", false, Some(C::PortInUse), None, &[], Some("Could not start the assistant server on port 7777: Address already in use (os error 98)")));
            },
        },
        Case {
            name: "flatpak-without-network",
            verdict: Yes,
            why: "the sandbox section says the network is not shared, next to the failing sync call",
            area: "Packaging",
            must: &[r"Network: no", r"sync/login FAIL[^\n]*class=(dns|unreachable)"],
            setup: |s| {
                s.flatpak = Some(FLATPAK_NO_NET.into());
                kosync_cfg(&mut s.cfg, "https://sync.example.org/koreader", false);
                s.trace.push(ev(Area::Sync, "login", false, Some(C::Dns), None, &[], Some("dns error: failed to lookup address information")));
            },
        },
        Case {
            name: "library-on-nfs",
            verdict: Yes,
            why: "the file system type and its kind are told, with the other folder facts",
            area: "Library folder",
            must: &[r"File system: nfs4", r"File system kind: NETWORK file system", r"Synced by: none"],
            setup: |s| {
                s.probe.mountinfo = "36 25 0:32 / /mnt/nas rw,relatime shared:1 - nfs4 server:/export rw\n".into();
                s.probe.canon = "/mnt/nas/Books".into();
            },
        },
        Case {
            name: "library-on-fuse-rclone",
            verdict: Yes,
            why: "the FUSE file system and its kind",
            area: "Library folder",
            must: &[r"File system: fuse\.rclone", r"File system kind: FUSE file system"],
            setup: |s| {
                s.probe.mountinfo = "40 25 0:40 / /home/u/Cloud rw,nosuid shared:2 - fuse.rclone remote: rw\n".into();
                s.probe.canon = "/home/u/Cloud/Books".into();
            },
        },
        Case {
            name: "disk-full-during-backup",
            verdict: Yes,
            why: "the backup fails with class disk_full and the free space class of the library disk",
            area: "Storage",
            must: &[r"backup/job FAIL[^\n]*class=disk_full", r"Free space: under 200 MB", r"Disk nearly full: yes"],
            setup: |s| {
                s.probe.free = Some((80 << 20, 500 << 30));
                s.trace.push(ev(Area::Backup, "job", false, Some(C::DiskFull), None, &[("kind", "backup")], Some("No space left on device (os error 28)")));
            },
        },
        Case {
            name: "keyring-locked",
            verdict: Yes,
            why: "the settings line says passwords are in a locked store",
            area: "Passwords and keyring",
            must: &[r"Passwords kept in: encrypted store \(locked\)"],
            setup: |s| {
                s.keyring = "encrypted store (locked)".into();
            },
        },
        Case {
            name: "keyring-missing",
            verdict: Yes,
            why: "the settings line says passwords are not saved, with the unlock attempt",
            area: "Passwords and keyring",
            must: &[r"Passwords kept in: not saved \(session only\)"],
            setup: |s| {
                s.keyring = "not saved (session only)".into();
            },
        },
        Case {
            name: "hindi-layout-complaint",
            verdict: Partly,
            why: "the script, the languages, the fonts and the renderer narrow it, but a wrong glyph or direction leaves no error to find",
            area: "Reader",
            must: &[r"Language script: Devanagari", r"Interface language: hi", r"Renderer: ngl"],
            setup: |s| {
                s.env.language = "hi".into();
            },
        },
        Case {
            name: "huge-library-startup-stall",
            verdict: Yes,
            why: "the start-up time, the stall events with the area and last operation, the library size and its disk",
            area: "Display and interface",
            must: &[r"Start-up: 14000 ms", r"ui/stall[^\n]*in=library", r"last_op=", r"Books: \d+"],
            setup: |s| {
                s.env.startup_ms = Some(14_000);
                crate::trace::set_active(Area::Library);
                let mut e = ev(Area::Ui, "stall", false, Some(C::Timeout), None, &[("in", "library"), ("area", "library"), ("last_op", "open_library")], None);
                e.ms = Some(3200);
                s.trace.push(e);
            },
        },
        Case {
            name: "dictionary-download-flaky",
            verdict: Yes,
            why: "the job fails with class timeout and says it was a dictionary download",
            area: "Metadata and lookup",
            must: &[r"metadata/job FAIL[^\n]*class=timeout", r"kind=dictionary_download"],
            setup: |s| {
                s.trace.push(ev(Area::Metadata, "job", false, Some(C::Timeout), None, &[("kind", "dictionary_download"), ("took_ms", "30000")], Some("operation timed out")));
            },
        },
        Case {
            name: "opds-server-401",
            verdict: Yes,
            why: "the request fails with 401, the kind of host and the shape of the catalogue address",
            area: "OPDS and web import",
            must: &[r"opds/request FAIL[^\n]*http=401", r"OPDS catalogue 1 address: https://<domain>/opds", r"host=domain"],
            setup: |s| {
                s.cfg.opds_catalogs.push(crate::opds_browse::Catalog { id: "c".into(), name: "x".into(), url: "https://books.example.org/opds".into(), user: "u".into(), password: "p".into() });
                s.trace.push(ev(Area::Opds, "request", false, Some(C::Unauthorized), Some(401), &[("method", "get"), ("scheme", "https"), ("host", "domain")], Some("HTTP 401")));
            },
        },
        Case {
            name: "calibre-import-unreadable-database",
            verdict: Yes,
            why: "the import fails with its class and a message that says the file is not a database",
            area: "Import and add books",
            must: &[r"import/calibre FAIL[^\n]*class=corrupt", r"not a database"],
            setup: |s| {
                s.trace.push(ev(Area::Import, "calibre", false, Some(C::Corrupt), None, &[], Some("file is not a database")));
            },
        },
        Case {
            name: "app-crashes-while-opening-a-pdf",
            verdict: Yes,
            why: "the crash text with the message and the first frames, and the last operations before it (the book's format and size)",
            area: "Crash",
            must: &[r"### Last crash", r"message: index out of bounds", r"reader/open_book[^\n]*fmt=pdf"],
            setup: |s| {
                s.trace.push(ev(Area::Reader, "open_book", true, None, None, &[("fmt", "pdf"), ("size", "lt100mb"), ("chapters", "1")], None));
                s.crash = Some("Bookx 0.3.0-beta.3 (beta) closed unexpectedly at 2026-11-23 10:00:00 UTC\nthread: main\nmessage: index out of bounds: the len is 0 but the index is 0\nat: src/ui/reader_pdf.rs:10:5\n\nbacktrace:\n   0: bookx::ui::reader_pdf::relayout\n".into());
            },
        },
        Case {
            name: "server-port-busy",
            verdict: Yes,
            why: "the start fails with class port_in_use and the port and sharing switches are listed",
            area: "Built-in server",
            must: &[r"server/start FAIL[^\n]*class=port_in_use", r"Port: 30260", r"Reachable from other devices: "],
            setup: |s| {
                s.cfg.share.port = 30260;
                s.trace.push(ev(Area::Server, "start", false, Some(C::PortInUse), None, &[("port", "30260"), ("https", "no"), ("lan", "yes")], Some("Could not start sharing on port 30260: Address already in use (os error 98)")));
            },
        },
        Case {
            name: "news-feed-404",
            verdict: Yes,
            why: "the news job fails with the HTTP status of the feed",
            area: "News and read later",
            must: &[r"news/job FAIL[^\n]*http=404", r"kind=news"],
            setup: |s| {
                s.trace.push(ev(Area::News, "job", false, Some(C::Status), Some(404), &[("kind", "news"), ("took_ms", "800")], Some("HTTP 404")));
            },
        },
        Case {
            name: "device-copy-disk-full",
            verdict: Yes,
            why: "the copy fails with class disk_full for the named device model and format",
            area: "Storage",
            must: &[r"devices/copy FAIL[^\n]*class=disk_full", r"device=Kobo_Libra_2", r"fmt=epub"],
            setup: |s| {
                s.trace.push(ev(Area::Devices, "copy", false, Some(C::DiskFull), None, &[("device", "Kobo_Libra_2"), ("backend", "usb"), ("fmt", "epub"), ("size", "lt10mb")], Some("No space left on device (os error 28)")));
            },
        },
        Case {
            name: "full-text-index-empty",
            verdict: Yes,
            why: "the library section says how many books are indexed, with the disk and file system of the folder",
            area: "Search",
            must: &[r"Search index: 0 of \d+ books", r"File system: ext4"],
            setup: |s| {
                s.indexed = Some((0, 120));
            },
        },
        Case {
            name: "library-folder-read-only",
            verdict: Yes,
            why: "the folder section says it is read-only",
            area: "Library folder",
            must: &[r"Read-only folder: yes"],
            setup: |s| {
                s.probe.read_only = true;
            },
        },
        Case {
            name: "library-inside-syncthing",
            verdict: Yes,
            why: "the folder section names the sync client",
            area: "Library folder",
            must: &[r"Synced by: Syncthing"],
            setup: |s| {
                s.probe.sync_client = Some("Syncthing".into());
            },
        },
        Case {
            name: "annotations-from-device-not-linked",
            verdict: Yes,
            why: "the merge event counts the annotations that found no book, with the device model and the sync switches",
            area: "Built-in server",
            must: &[r"server/annotation_merge[^\n]*unplaced_or_unlinked=4", r"Annotations from KOReader: on"],
            setup: |s| {
                s.cfg.share.annotations = true;
                s.trace.push(ev(Area::Server, "annotation_merge", true, None, None, &[("added", "0"), ("notes", "0"), ("unplaced_or_unlinked", "4")], None));
            },
        },
        Case {
            name: "opds-lan-server-timeout",
            verdict: Yes,
            why: "the request times out; the host is told as a private address and the scheme is plain http",
            area: "OPDS and web import",
            must: &[r"opds/request FAIL[^\n]*class=timeout", r"host=ip_private", r"scheme=http\b"],
            setup: |s| {
                s.trace.push(ev(Area::Opds, "request", false, Some(C::Timeout), None, &[("method", "get"), ("scheme", "http"), ("host", "ip_private")], Some("timed out reading response")));
            },
        },
        Case {
            name: "import-of-a-damaged-epub",
            verdict: Yes,
            why: "the import fails with class corrupt, the format and the size class",
            area: "Import and add books",
            must: &[r"import/prepare FAIL[^\n]*class=corrupt", r"fmt=epub", r"size=lt10mb"],
            setup: |s| {
                s.trace.push(ev(Area::Import, "prepare", false, Some(C::Corrupt), None, &[("fmt", "epub"), ("size", "lt10mb")], Some("the file is not a zip archive")));
            },
        },
        Case {
            name: "metadata-service-rate-limited",
            verdict: Yes,
            why: "the request fails with HTTP 429 and the kind of host",
            area: "Metadata and lookup",
            must: &[r"metadata/request FAIL[^\n]*http=429", r"host=domain"],
            setup: |s| {
                s.trace.push(ev(Area::Metadata, "request", false, Some(C::Status), Some(429), &[("method", "get"), ("scheme", "https"), ("host", "domain")], Some("HTTP 429")));
            },
        },
        Case {
            name: "settings-file-cannot-be-saved",
            verdict: Yes,
            why: "the save fails with its class (read-only or full disk) and the folder facts",
            area: "Settings",
            must: &[r"settings/save FAIL[^\n]*class=denied", r"Read-only folder: no"],
            setup: |s| {
                s.flatpak = None;
                s.trace.push(ev(Area::Settings, "save", false, Some(C::Denied), None, &[], Some("Permission denied (os error 13)")));
            },
        },
    ]
}

pub fn build_report(case: &Case, opts: Options) -> String {
    let dir = crate::diag_pii::tmp(case.name);
    let mut lib = crate::core::library::Library::in_memory(&dir.join("lib")).unwrap();
    let p = dir.join("book.md");
    std::fs::write(&p, "---\ntitle: \"A Plain Book\"\n---\n# A\n\nHi.\n").unwrap();
    lib.add_path(&p, true).unwrap();
    let id = lib.books[0].id;
    lib.books[0].file = "book.pdf".into();
    lib.books[0].size = 3 << 20;
    let mut sc = base();
    sc.book = id;
    (case.setup)(&mut sc);
    let plugins = if case.name == "plugin-fails-to-load" {
        vec![bookx_plugin::PluginStatus { id: "acme.trim".into(), name: "Trim".into(), version: "1.0.0".into(), kind: None, description: String::new(), dir: "/x".into(), permissions: vec![], actions: vec![], enabled: true, loaded: false, error: Some("the module traps".into()) }]
    } else {
        vec![]
    };
    let w = World {
        env: sc.env.clone(),
        cfg: &sc.cfg,
        lib: &lib,
        indexed: sc.indexed,
        plugins,
        patches: 0,
        keyring: sc.keyring.clone(),
        server: sc.server.clone(),
        jobs: vec![],
        log: sc.log.clone(),
        trace: sc.trace.clone(),
        previous: vec![],
        crash: sc.crash.clone(),
        flatpak_info: sc.flatpak.clone(),
        backend: "wayland".into(),
        renderer: "ngl".into(),
        vars: Default::default(),
        extra: sc.extra.clone(),
        entry: None,
        probe: Some(sc.probe.clone()),
        helpers: sc.helpers,
    };
    let input = crate::diag_gather::input(&w);
    let scrub = crate::debug_log::Scrubber::new(crate::diag_gather::scrub_rules(&sc.cfg, &lib, vec![], ""));
    let r = report::build(&input, opts, "", &scrub);
    let _ = std::fs::remove_dir_all(dir);
    r
}

/// The area the report's summary names.
pub fn summary_area(report: &str) -> Option<String> {
    report
        .lines()
        .find_map(|l| l.strip_prefix("- Suspected area: "))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[test]
    fn every_case_is_found_by_the_summary_and_has_the_facts_the_maintainer_needs() {
        let mut failures = Vec::new();
        for c in cases() {
            if c.verdict == Verdict::No {
                continue;
            }
            let r = build_report(&c, Options::default());
            let got = summary_area(&r).unwrap_or_default();
            if got != c.area {
                failures.push(format!("{}: area `{got}`, wanted `{}`", c.name, c.area));
            }
            for m in c.must {
                if !Regex::new(m).unwrap().is_match(&r) {
                    failures.push(format!("{}: `{m}` is not in the report", c.name));
                }
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn at_least_ninety_percent_of_the_cases_are_identified_from_the_report_alone() {
        let all = cases();
        let yes = all.iter().filter(|c| c.verdict == Verdict::Yes).count();
        assert!(all.len() >= 25, "{} cases", all.len());
        assert!(yes * 100 >= all.len() * 90, "{yes} of {}", all.len());
    }

    #[test]
    fn no_case_leaks_a_personal_value_or_needs_an_opt_in() {
        for c in cases() {
            if c.verdict == Verdict::No {
                continue;
            }
            let r = build_report(&c, Options::default());
            assert!(!r.contains("A Plain Book"), "{}: the title is in the report", c.name);
            assert!(!r.contains("sync.example.org") && !r.contains("192.168.1.5") && !r.contains("nas.example.org"), "{}", c.name);
        }
    }

    /// The Python triage tool and the report's own summary read one table: they must agree.
    #[test]
    fn the_python_tool_and_the_summary_block_name_the_same_cause() {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/build-aux/triage-report.py");
        let Ok(probe) = std::process::Command::new("python3").arg("--version").output() else {
            return; // no Python here: the CI job that has it runs this
        };
        if !probe.status.success() {
            return;
        }
        for c in cases().iter().filter(|c| c.verdict != Verdict::No) {
            let r = build_report(c, Options::default());
            let want = crate::triage::headline(&crate::triage::analyse(&r)).map(|f| (f.id.clone(), f.area.clone()));
            let mut child = std::process::Command::new("python3")
                .args([script, "--json"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            use std::io::Write;
            child.stdin.take().unwrap().write_all(r.as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            let got = v["headline"]["id"].as_str().map(|id| (id.to_string(), v["headline"]["area"].as_str().unwrap_or("").to_string()));
            assert_eq!(got, want, "{}", c.name);
        }
    }

    #[test]
    fn the_terminal_output_of_a_package_for_another_release_is_recognised() {
        let t = "bookx: /lib64/libc.so.6: version `GLIBC_2.43' not found (required by bookx)\nbookx: error while loading shared libraries: libcrypto.so.3: cannot open shared object file";
        let f = crate::triage::analyse(t);
        assert_eq!(crate::triage::headline(&f).unwrap().id, "package-binary");
    }

    #[test]
    fn the_coverage_table_in_the_notes_lists_every_case_with_its_verdict() {
        let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/diagnostic-report-notes.md")).unwrap_or_default();
        for c in cases() {
            let v = match c.verdict {
                Verdict::Yes => "yes",
                Verdict::Partly => "partly",
                Verdict::No => "no",
            };
            let row = format!("| `{}` | {v} |", c.name);
            assert!(doc.contains(&row), "docs/diagnostic-report-notes.md has no row `{row}`");
        }
    }
}
