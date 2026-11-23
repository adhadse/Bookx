//! Facts about the machine, the package and the library folder, for the report.
//!
//! The parsers are pure functions of the text they read (`/.flatpak-info`, `/proc/meminfo`,
//! `/proc/self/mountinfo`, `/proc/self/status`) so the tests feed them made-up text. The `*_section`
//! functions read the real files and return a [`Section`]; every value in it is a [`Fact`], so the
//! redaction rules of [`crate::diag_facts`] apply without this module having to remember them.
//!
//! Numbers that could identify a person or a machine (RAM, free space, the size of a database) are
//! told as classes. No GTK, no unsafe (free space comes from `rustix::fs::statvfs`).

use std::path::Path;

use crate::diag_facts::{Fact, Section};

// ---- Flatpak ----

/// What `/.flatpak-info` says about the sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlatpakInfo {
    /// `org.gnome.Platform/x86_64/51`
    pub runtime: String,
    pub flatpak_version: String,
    pub branch: String,
    pub shared: Vec<String>,
    pub sockets: Vec<String>,
    pub devices: Vec<String>,
    pub features: Vec<String>,
    /// Sanitised: `home`, `xdg-download`, `<path>`; a leading `!` (a removed permission) is kept.
    pub filesystems: Vec<String>,
    /// Permissions the user changed (an override exists) in `[Context]`.
    pub has_overrides: bool,
}

fn list(v: &str) -> Vec<String> {
    v.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Filesystem permissions that name no user path.
const FS_WORDS: &[&str] = &[
    "home",
    "host",
    "host-os",
    "host-etc",
    "host-root",
    "xdg-desktop",
    "xdg-documents",
    "xdg-download",
    "xdg-music",
    "xdg-pictures",
    "xdg-public-share",
    "xdg-templates",
    "xdg-videos",
    "xdg-run",
    "xdg-config",
    "xdg-cache",
    "xdg-data",
    "/media",
    "/run/media",
    "/mnt",
    "/tmp",
    "/var/tmp",
    "/usr",
    "/etc",
];

/// One `filesystems` entry without the user's folder names: `~/Books:rw` becomes `<path>:rw`.
pub fn sanitize_filesystem(entry: &str) -> String {
    let (neg, rest) = match entry.strip_prefix('!') {
        Some(r) => ("!", r),
        None => ("", entry),
    };
    let (path, mode) = match rest.rsplit_once(':') {
        Some((p, m)) if matches!(m, "ro" | "rw" | "create") => (p, Some(m)),
        _ => (rest, None),
    };
    // `xdg-download/Books` keeps its first part
    let first = path.split('/').next().unwrap_or("");
    let shown = if FS_WORDS.contains(&path) {
        path.to_string()
    } else if FS_WORDS.contains(&first) && !first.is_empty() && first.starts_with("xdg-") {
        format!("{first}/<path>")
    } else {
        "<path>".to_string()
    };
    match mode {
        Some(m) => format!("{neg}{shown}:{m}"),
        None => format!("{neg}{shown}"),
    }
}

pub fn parse_flatpak_info(text: &str) -> FlatpakInfo {
    let mut section = String::new();
    let mut info = FlatpakInfo::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(s) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            section = s.to_string();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match (section.as_str(), k) {
            ("Application", "runtime") => info.runtime = v.to_string(),
            ("Instance", "flatpak-version") => info.flatpak_version = v.to_string(),
            ("Instance", "branch") => info.branch = v.to_string(),
            ("Context", "shared") => info.shared = list(v),
            ("Context", "sockets") => info.sockets = list(v),
            ("Context", "devices") => info.devices = list(v),
            ("Context", "features") => info.features = list(v),
            ("Context", "filesystems") => {
                info.filesystems = list(v).iter().map(|e| sanitize_filesystem(e)).collect()
            }
            ("Instance", "instance-id") | ("Instance", "instance-path") => {}
            ("Instance", "app-extensions") | ("Instance", "runtime-extensions") => {}
            _ => {}
        }
    }
    // a `!` entry removes a permission the manifest gave: only a user override does that
    info.has_overrides = info.filesystems.iter().any(|f| f.starts_with('!'));
    info
}

pub fn flatpak_section(text: &str) -> Section {
    let i = parse_flatpak_info(text);
    let mut s = Section::new("Flatpak sandbox");
    let runtime = i.runtime.rsplit('/').next().unwrap_or("");
    let parts: Vec<&str> = i.runtime.split('/').collect();
    s.row(
        "Runtime",
        Fact::choice(
            parts
                .get(if parts.first() == Some(&"runtime") {
                    1
                } else {
                    0
                })
                .copied()
                .unwrap_or(""),
            &[
                "org.gnome.Platform",
                "org.freedesktop.Platform",
                "org.kde.Platform",
            ],
        ),
    )
    .row("Runtime branch", Fact::version(runtime))
    .row("Flatpak", Fact::version(&i.flatpak_version))
    .row(
        "Network",
        Fact::flag(i.shared.iter().any(|x| x == "network")),
    )
    .row(
        "Sockets",
        Fact::choices(
            &i.sockets,
            &[
                "wayland",
                "fallback-x11",
                "x11",
                "pulseaudio",
                "session-bus",
                "system-bus",
                "ssh-auth",
                "pcsc",
                "cups",
                "gpg-agent",
                "inherit-wayland-socket",
            ],
        ),
    )
    .row(
        "Devices",
        Fact::choices(&i.devices, &["dri", "all", "kvm", "shm", "input", "usb"]),
    )
    .row(
        "Features",
        Fact::choices(
            &i.features,
            &[
                "devel",
                "multiarch",
                "bluetooth",
                "canbus",
                "per-app-dev-shm",
            ],
        ),
    );
    let fs: Vec<String> = i.filesystems.clone();
    s.row("File access", Fact::choices_by(&fs, &is_sanitized_fs));
    s
}

/// Whether `e` has the form [`sanitize_filesystem`] produces (re-checked where it is shown).
pub fn is_sanitized_fs(e: &str) -> bool {
    let e = e.strip_prefix('!').unwrap_or(e);
    let e = ["ro", "rw", "create"]
        .iter()
        .find_map(|m| e.strip_suffix(&format!(":{m}")))
        .unwrap_or(e);
    e == "<path>"
        || FS_WORDS.contains(&e)
        || e.strip_suffix("/<path>")
            .is_some_and(|w| w.starts_with("xdg-") && FS_WORDS.contains(&w))
}

// ---- memory, CPU, process ----

/// RAM as a class from the text of `/proc/meminfo`.
pub fn ram_class(meminfo: &str) -> &'static str {
    let kb = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse::<u64>().ok());
    match kb.map(|k| k / (1024 * 1024)) {
        None => "unknown",
        Some(0..=1) => "up to 2 GB",
        Some(2..=3) => "2-4 GB",
        Some(4..=7) => "4-8 GB",
        Some(8..=15) => "8-16 GB",
        Some(16..=31) => "16-32 GB",
        Some(_) => "32 GB or more",
    }
}

/// The resident memory of this process in KB, from `/proc/self/status`.
pub fn rss_kb(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

pub fn rss_class(kb: Option<u64>) -> &'static str {
    match kb.map(|k| k / 1024) {
        None => "unknown",
        Some(0..=199) => "under 200 MB",
        Some(200..=499) => "200-500 MB",
        Some(500..=999) => "500 MB-1 GB",
        Some(1000..=1999) => "1-2 GB",
        Some(_) => "over 2 GB",
    }
}

/// The share of one CPU this process used since it started, as a class, from `/proc/self/stat` and the
/// uptime of the process in seconds.
pub fn cpu_class(stat: &str, hz: u64, uptime_s: f64) -> &'static str {
    // fields after the `)` of the command name: state is field 3, utime 14, stime 15 (1-based)
    let after = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let f: Vec<&str> = after.split_whitespace().collect();
    let ticks: u64 = [11, 12]
        .iter()
        .filter_map(|i| f.get(*i).and_then(|v| v.parse::<u64>().ok()))
        .sum();
    if hz == 0 || uptime_s < 1.0 || f.len() < 13 {
        return "unknown";
    }
    let share = ticks as f64 / hz as f64 / uptime_s;
    match share {
        s if s < 0.05 => "idle (under 5%)",
        s if s < 0.25 => "light (5-25%)",
        s if s < 0.75 => "busy (25-75%)",
        _ => "very busy (over 75%)",
    }
}

pub fn free_class(avail: u64, total: u64) -> (&'static str, bool) {
    let pct = if total == 0 { 100 } else { avail * 100 / total };
    let nearly = avail < 200 << 20 || pct < 2;
    let c = match avail {
        a if a < 200 << 20 => "under 200 MB",
        a if a < 1 << 30 => "200 MB-1 GB",
        a if a < 10 << 30 => "1-10 GB",
        a if a < 100 << 30 => "10-100 GB",
        _ => "over 100 GB",
    };
    (c, nearly)
}

/// Available and total bytes of the file system that holds `path`.
pub fn free_space(path: &Path) -> Option<(u64, u64)> {
    let s = rustix::fs::statvfs(path).ok()?;
    Some((s.f_bavail * s.f_frsize, s.f_blocks * s.f_frsize))
}

// ---- file systems ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsClass {
    Local,
    Network,
    Fuse,
    Removable,
    Portal,
    Overlay,
    Unknown,
}

/// What kind of file system a type is; the second value is the type itself when it is a known word.
pub fn fs_class(fstype: &str) -> FsClass {
    let t = fstype.to_ascii_lowercase();
    if t.starts_with("nfs")
        || [
            "cifs",
            "smb3",
            "smbfs",
            "afs",
            "ceph",
            "glusterfs",
            "9p",
            "ncpfs",
            "davfs",
            "lustre",
        ]
        .contains(&t.as_str())
    {
        FsClass::Network
    } else if t == "fuse.portal" {
        FsClass::Portal
    } else if t.starts_with("fuse") {
        FsClass::Fuse
    } else if [
        "vfat", "exfat", "ntfs", "ntfs3", "msdos", "hfsplus", "udf", "iso9660",
    ]
    .contains(&t.as_str())
    {
        FsClass::Removable
    } else if t == "overlay" {
        FsClass::Overlay
    } else if [
        "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "zfs", "tmpfs", "bcachefs", "jfs",
        "reiserfs",
    ]
    .contains(&t.as_str())
    {
        FsClass::Local
    } else {
        FsClass::Unknown
    }
}

pub const FS_TYPES: &[&str] = &[
    "ext2",
    "ext3",
    "ext4",
    "btrfs",
    "xfs",
    "f2fs",
    "zfs",
    "tmpfs",
    "bcachefs",
    "jfs",
    "vfat",
    "exfat",
    "ntfs",
    "ntfs3",
    "overlay",
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "9p",
    "ceph",
    "glusterfs",
    "davfs",
    "fuse.portal",
    "fuse.sshfs",
    "fuse.rclone",
    "fuse.gvfsd-fuse",
    "fuseblk",
    "fuse.mergerfs",
    "fuse.encfs",
    "fuse.cryfs",
    "fuse.gocryptfs",
    "fuse",
    "hfsplus",
    "udf",
    "iso9660",
];

pub fn fs_class_word(c: FsClass) -> &'static str {
    match c {
        FsClass::Local => "local disk",
        FsClass::Network => "NETWORK file system",
        FsClass::Fuse => "FUSE file system",
        FsClass::Removable => "removable-media file system",
        FsClass::Portal => "document portal",
        FsClass::Overlay => "overlay",
        FsClass::Unknown => "unknown",
    }
}

// ---- gathering ----

fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// Environment variables that only say *whether* a proxy is configured.
pub fn proxy_set(var: &dyn Fn(&str) -> String) -> bool {
    [
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ]
    .iter()
    .any(|k| !var(k).is_empty())
}

/// Text direction and script of the language the app is shown in.
pub fn script_of(lang: &str) -> &'static str {
    match lang.split(['_', '-']).next().unwrap_or("") {
        "ar" | "fa" | "ur" | "ps" | "sd" | "ug" => "right-to-left (Arabic script)",
        "he" | "yi" => "right-to-left (Hebrew script)",
        "hi" | "mr" | "ne" | "sa" => "Devanagari",
        "bn" | "as" => "Bengali",
        "ta" | "te" | "kn" | "ml" | "gu" | "pa" | "si" | "th" | "lo" | "km" | "my" => {
            "other Indic or South-East Asian"
        }
        "zh" | "ja" | "ko" => "CJK",
        "ru" | "uk" | "bg" | "sr" | "be" | "mk" | "kk" => "Cyrillic",
        "el" => "Greek",
        "" => "unknown",
        _ => "Latin",
    }
}

/// The machine: processors, memory, time zone, proxy, display.
pub fn machine_section(display: &str, renderer: &str, var: &dyn Fn(&str) -> String) -> Section {
    let mut s = Section::new("Machine");
    s.row(
        "Processors",
        Fact::num(std::thread::available_parallelism().map_or(0, |n| n.get() as u64)),
    )
    .row(
        "Memory",
        Fact::choice(
            ram_class(&read("/proc/meminfo")),
            &[
                "unknown",
                "up to 2 GB",
                "2-4 GB",
                "4-8 GB",
                "8-16 GB",
                "16-32 GB",
                "32 GB or more",
            ],
        ),
    )
    .row(
        "Time zone offset",
        Fact::utc_offset(chrono::Local::now().offset().local_minus_utc() / 60),
    )
    .row(
        "Display backend",
        Fact::choice(
            display,
            &["wayland", "x11", "broadway", "macos", "windows", "other"],
        ),
    )
    .row(
        "Renderer",
        Fact::choice(renderer, &["ngl", "vulkan", "gl", "cairo", "unknown"]),
    )
    .row("Proxy variables set", Fact::flag(proxy_set(var)))
    .row("GSK_RENDERER", {
        let g = var("GSK_RENDERER");
        if g.is_empty() {
            Fact::word("not set")
        } else {
            Fact::choice(&g, &["ngl", "vulkan", "gl", "cairo"])
        }
    });
    s
}

/// Memory and CPU of this process, sampled now.
pub fn process_section(started: Option<std::time::Instant>) -> Section {
    let mut s = Section::new("This run");
    s.row(
        "Memory of Bookx",
        Fact::choice(
            rss_class(rss_kb(&read("/proc/self/status"))),
            &[
                "unknown",
                "under 200 MB",
                "200-500 MB",
                "500 MB-1 GB",
                "1-2 GB",
                "over 2 GB",
            ],
        ),
    );
    if let Some(t) = started {
        let up = t.elapsed().as_secs_f64();
        s.row("Running for (minutes)", Fact::count_class(up as u64 / 60));
        s.row(
            "CPU used since start",
            Fact::choice(
                cpu_class(&read("/proc/self/stat"), 100, up),
                &[
                    "unknown",
                    "idle (under 5%)",
                    "light (5-25%)",
                    "busy (25-75%)",
                    "very busy (over 75%)",
                ],
            ),
        );
    }
    s
}

/// What was read about the library folder (the tests make one by hand).
#[derive(Debug, Clone, Default)]
pub struct LibraryProbe {
    pub mountinfo: String,
    pub canon: std::path::PathBuf,
    pub free: Option<(u64, u64)>,
    pub sync_client: Option<String>,
    pub db: Option<DbProbe>,
    pub read_only: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DbProbe {
    pub size: u64,
    /// Files such as `bookx-library.db.pre-v8-...` beside the catalogue.
    pub copies: usize,
    /// Problems the quick check found, `None` when it was not run (a large file).
    pub problems: Option<usize>,
}

/// Reads the folder: file system, free space, sync client, the catalogue.
pub fn probe(root: &Path) -> LibraryProbe {
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let db_path = bookx_store::library_db_path(root);
    let db = std::fs::metadata(&db_path).ok().map(|m| DbProbe {
        size: m.len(),
        copies: std::fs::read_dir(root)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_name().to_string_lossy().contains(".pre-v"))
                    .count()
            })
            .unwrap_or(0),
        problems: (m.len() < 200 << 20)
            .then(|| bookx_store::SqliteStore::check_file(&db_path).len()),
    });
    LibraryProbe {
        mountinfo: read("/proc/self/mountinfo"),
        free: free_space(&canon),
        sync_client: bookx_store::guard::sync_client(&canon, &|p| p.exists()).map(str::to_string),
        db,
        read_only: root
            .metadata()
            .map(|m| m.permissions().readonly())
            .unwrap_or(false),
        canon,
    }
}

/// The library folder: file system, free space, sync clients, the catalogue and its copies.
pub fn library_fs_section(p: &LibraryProbe) -> Section {
    let mut s = Section::new("Library folder");
    match bookx_store::guard::fs_type_for(&p.mountinfo, &p.canon) {
        Some(t) => {
            let c = fs_class(&t);
            s.row("File system", Fact::choice(&t, FS_TYPES)).row(
                "File system kind",
                Fact::choice(
                    fs_class_word(c),
                    &[
                        "local disk",
                        "NETWORK file system",
                        "FUSE file system",
                        "removable-media file system",
                        "document portal",
                        "overlay",
                        "unknown",
                    ],
                ),
            );
        }
        None => {
            s.row("File system", Fact::word("unknown"));
        }
    }
    s.row(
        "Synced by",
        Fact::choice(
            p.sync_client.as_deref().unwrap_or("none"),
            &[
                "none",
                "Dropbox",
                "Syncthing",
                "Resilio Sync",
                "ownCloud/Nextcloud",
                "Nextcloud",
                "OneDrive",
                "Google Drive",
                "ownCloud",
                "iCloud Drive",
            ],
        ),
    );
    match p.free {
        Some((avail, total)) => {
            let (c, nearly) = free_class(avail, total);
            s.row(
                "Free space",
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
            );
            s.row("Disk nearly full", Fact::flag(nearly));
        }
        None => {
            s.row("Free space", Fact::word("unknown"));
        }
    }
    match &p.db {
        Some(d) => {
            s.row("Catalogue size", Fact::size(d.size));
            s.row("Pre-update copies of the catalogue", Fact::count(d.copies));
            s.row(
                "Catalogue integrity (quick check)",
                match d.problems {
                    Some(0) => Fact::word("ok"),
                    Some(n) => Fact::text(&format!("{n} problem(s)")),
                    None => Fact::word("skipped (large file)"),
                },
            );
        }
        None => {
            s.row("Catalogue", Fact::word("not found in the library folder"));
        }
    }
    s.row("Read-only folder", Fact::flag(p.read_only));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "[Application]\nname=com.adhadse.Bookx\nruntime=runtime/org.gnome.Platform/x86_64/51\n\n[Instance]\ninstance-id=123456\ninstance-path=/home/alice/.var/app/com.adhadse.Bookx\nflatpak-version=1.16.1\nbranch=stable\n\n[Context]\nshared=network;ipc;\nsockets=wayland;fallback-x11;pulseaudio;\ndevices=dri;\nfilesystems=xdg-download;/home/alice/Books:rw;!host;~/Secret Folder;xdg-documents/Private Stuff:ro;\n";

    #[test]
    fn the_sandbox_is_read_without_the_users_folders() {
        let i = parse_flatpak_info(INFO);
        assert_eq!(i.runtime, "runtime/org.gnome.Platform/x86_64/51");
        assert_eq!(i.flatpak_version, "1.16.1");
        assert_eq!(
            i.filesystems,
            [
                "xdg-download",
                "<path>:rw",
                "!host",
                "<path>",
                "xdg-documents/<path>:ro"
            ]
        );
        assert!(i.has_overrides);
        let t = crate::diag_facts::Redactor {
            scrub: &crate::debug_log::Scrubber::new(Default::default()),
            show_host: false,
            show_titles: false,
        }
        .section(&flatpak_section(INFO));
        for bad in ["alice", "Secret", "Private", "123456", "/home"] {
            assert!(!t.contains(bad), "{bad}: {t}");
        }
        assert!(t.contains("- Network: yes") && t.contains("- Runtime: org.gnome.Platform"));
        assert!(t.contains("wayland, fallback-x11, pulseaudio"));
    }

    #[test]
    fn a_sandbox_without_network_says_so() {
        let t = "[Context]\nshared=ipc;\nsockets=wayland;\n";
        let s = flatpak_section(t);
        assert!(s.rows.iter().any(|(k, _)| k == "Network"));
        assert_eq!(parse_flatpak_info(t).shared, ["ipc"]);
    }

    #[test]
    fn filesystem_entries_keep_only_words() {
        for (e, want) in [
            ("home", "home"),
            ("home:ro", "home:ro"),
            ("!host", "!host"),
            ("/media", "/media"),
            ("/mnt/NAS-Alice:rw", "<path>:rw"),
            ("xdg-music/Alice's Albums", "xdg-music/<path>"),
            ("~/Books", "<path>"),
        ] {
            assert_eq!(sanitize_filesystem(e), want, "{e}");
        }
    }

    #[test]
    fn memory_is_told_as_a_class() {
        let m = |kb: u64| format!("MemTotal:       {kb} kB\nMemFree: 1 kB\n");
        assert_eq!(ram_class(&m(1_000_000)), "up to 2 GB");
        assert_eq!(ram_class(&m(3_800_000)), "2-4 GB");
        assert_eq!(ram_class(&m(7_900_000)), "4-8 GB");
        assert_eq!(ram_class(&m(17_000_000)), "16-32 GB");
        assert_eq!(ram_class(&m(130_000_000)), "32 GB or more");
        assert_eq!(ram_class(""), "unknown");
        assert_eq!(
            rss_class(rss_kb("Name: x\nVmRSS:   523000 kB\n")),
            "500 MB-1 GB"
        );
        assert_eq!(rss_class(None), "unknown");
    }

    #[test]
    fn cpu_use_is_told_as_a_class() {
        // pid (comm) state ppid ... utime(14) stime(15)
        let stat = |u: u64, s: u64| {
            format!(
                "42 (bookx) S 1 1 1 0 -1 4194560 100 0 0 0 {u} {s} 0 0 20 0 5 0 100 1000000 2000"
            )
        };
        assert_eq!(cpu_class(&stat(10, 5), 100, 100.0), "idle (under 5%)");
        assert_eq!(cpu_class(&stat(1000, 500), 100, 100.0), "light (5-25%)");
        assert_eq!(
            cpu_class(&stat(7000, 3000), 100, 100.0),
            "very busy (over 75%)"
        );
        assert_eq!(cpu_class("garbage", 100, 100.0), "unknown");
        assert_eq!(cpu_class(&stat(1, 1), 100, 0.0), "unknown");
        // a command name with a space and a parenthesis cannot confuse the field count
        let odd =
            "42 (a) b (c) S 1 1 1 0 -1 4194560 100 0 0 0 5000 0 0 0 20 0 5 0 100 1000000 2000";
        assert_ne!(cpu_class(odd, 100, 100.0), "unknown");
    }

    #[test]
    fn free_space_has_classes_and_a_nearly_full_flag() {
        assert_eq!(free_class(50 << 20, 100 << 30), ("under 200 MB", true));
        assert_eq!(free_class(5 << 30, 10 << 30), ("1-10 GB", false));
        assert!(!free_class(5 << 30, 100 << 30).1);
        assert_eq!(free_class(2 << 30, 500 << 30), ("1-10 GB", true));
        assert!(free_space(&std::env::temp_dir()).is_some());
    }

    #[test]
    fn file_systems_are_classed() {
        for (t, c) in [
            ("ext4", FsClass::Local),
            ("nfs4", FsClass::Network),
            ("cifs", FsClass::Network),
            ("fuse.sshfs", FsClass::Fuse),
            ("fuse.portal", FsClass::Portal),
            ("exfat", FsClass::Removable),
            ("overlay", FsClass::Overlay),
            ("weirdfs", FsClass::Unknown),
        ] {
            assert_eq!(fs_class(t), c, "{t}");
        }
    }

    #[test]
    fn the_library_folder_section_reads_the_real_disk() {
        let d = std::env::temp_dir().join(format!("bookx-diagfs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("bookx-library.db.pre-v8-20260101T000000"), b"x").unwrap();
        let s = library_fs_section(&probe(&d));
        let get = |k: &str| s.rows.iter().any(|(l, _)| l == k);
        assert!(get("File system") && get("Free space") && get("Synced by"));
        assert!(get("Catalogue"), "no database yet");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_library_on_nfs_with_a_full_disk_is_told_so() {
        let p = LibraryProbe {
            mountinfo: "36 25 0:32 / /mnt/nas rw,relatime shared:1 - nfs4 server:/export rw\n"
                .into(),
            canon: "/mnt/nas/Books".into(),
            free: Some((50 << 20, 500 << 30)),
            db: Some(DbProbe {
                size: 3 << 20,
                copies: 2,
                problems: Some(0),
            }),
            ..Default::default()
        };
        let t = crate::diag_facts::Redactor {
            scrub: &crate::debug_log::Scrubber::new(Default::default()),
            show_host: false,
            show_titles: false,
        }
        .section(&library_fs_section(&p));
        for want in [
            "File system: nfs4",
            "File system kind: NETWORK file system",
            "Free space: under 200 MB",
            "Disk nearly full: yes",
            "Pre-update copies of the catalogue: 2",
            "quick check): ok",
        ] {
            assert!(t.contains(want), "{want} in {t}");
        }
        assert!(!t.contains("server:/export") && !t.contains("/mnt/nas"));
    }

    #[test]
    fn scripts_and_directions_are_told() {
        assert_eq!(script_of("hi_IN"), "Devanagari");
        assert!(script_of("ar").starts_with("right-to-left"));
        assert_eq!(script_of("de"), "Latin");
        assert_eq!(script_of(""), "unknown");
    }

    #[test]
    fn a_proxy_is_only_told_as_set_or_not() {
        assert!(proxy_set(&|k| if k == "https_proxy" {
            "http://u:p@proxy:3128".into()
        } else {
            String::new()
        }));
        assert!(!proxy_set(&|_| String::new()));
    }
}
