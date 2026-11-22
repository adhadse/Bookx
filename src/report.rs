//! "Report a Problem": the Markdown a user pastes into a GitHub issue.
//!
//! [`build`] is a pure function of an [`Input`] (the environment, a settings summary, library statistics,
//! recent jobs, the recent log and the last crash text), so the tests inject a fixed `Environment` and
//! compare against a golden text. Everything that came from the machine is cleaned by the scrubber
//! (`debug_log::Scrubber`: paths, the library folder, titles, servers, accounts, secrets, addresses)
//! before it is put in; the text the user types under "What happened" is added unchanged, after a rule,
//! and is never written to the log. Nothing here touches the network.

use std::collections::BTreeMap;

use crate::debug_log::Scrubber;

/// The report is cut to this many bytes.
pub const MAX_BYTES: usize = 60 * 1024;
/// The user's own text is cut to this many bytes.
pub const MAX_USER_BYTES: usize = 8 * 1024;
/// Log lines in a report.
pub const LOG_LINES: usize = 200;
/// Longest job error shown.
const JOB_ERROR_BYTES: usize = 200;
/// Jobs listed.
pub const JOB_LINES: usize = 10;

/// How Bookx was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packaging {
    Flatpak,
    AppImage,
    Deb,
    Rpm,
    SystemPackage,
    Tarball,
}

/// What [`Packaging::detect_from`] looks at.
#[derive(Debug, Clone, Default)]
pub struct PackagingProbe {
    pub flatpak_id: bool,
    pub flatpak_info: bool,
    pub appimage: bool,
    pub exe: String,
    /// `ID` and `ID_LIKE` of os-release, lower case, space separated.
    pub os_ids: String,
}

impl Packaging {
    pub fn detect_from(p: &PackagingProbe) -> Packaging {
        if p.flatpak_id || p.flatpak_info || p.exe.starts_with("/app/") {
            Packaging::Flatpak
        } else if p.appimage {
            Packaging::AppImage
        } else if p.exe.starts_with("/usr/") || p.exe.starts_with("/opt/") {
            let ids: Vec<&str> = p.os_ids.split_whitespace().collect();
            if ids.iter().any(|i| ["debian", "ubuntu"].contains(i)) {
                Packaging::Deb
            } else if ids
                .iter()
                .any(|i| ["fedora", "rhel", "centos", "suse", "opensuse"].contains(i))
            {
                Packaging::Rpm
            } else {
                Packaging::SystemPackage
            }
        } else {
            Packaging::Tarball
        }
    }

    pub fn detect() -> Packaging {
        let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        Packaging::detect_from(&PackagingProbe {
            flatpak_id: std::env::var_os("FLATPAK_ID").is_some(),
            flatpak_info: std::path::Path::new("/.flatpak-info").exists(),
            appimage: std::env::var_os("APPIMAGE").is_some(),
            exe: std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            os_ids: os_ids(&os),
        })
    }
}

/// "Flatpak", "deb package", ...
pub fn packaging_text(p: &Packaging) -> &'static str {
    match p {
        Packaging::Flatpak => "Flatpak",
        Packaging::AppImage => "AppImage",
        Packaging::Deb => "deb package",
        Packaging::Rpm => "rpm package",
        Packaging::SystemPackage => "system package",
        Packaging::Tarball => "tarball or built from source",
    }
}

fn os_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        l.strip_prefix(key)
            .and_then(|r| r.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
    })
}

/// `PRETTY_NAME` of an os-release file.
pub fn pretty_name(os_release: &str) -> Option<String> {
    os_value(os_release, "PRETTY_NAME").filter(|s| !s.is_empty())
}

/// `ID` and `ID_LIKE` of an os-release file.
pub fn os_ids(os_release: &str) -> String {
    format!(
        "{} {}",
        os_value(os_release, "ID").unwrap_or_default(),
        os_value(os_release, "ID_LIKE").unwrap_or_default()
    )
    .to_ascii_lowercase()
}

/// Everything about the machine and the build. Tests make one by hand.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub version: String,
    pub channel: String,
    pub commit: String,
    pub profile: String,
    pub packaging: String,
    pub os: String,
    pub kernel: String,
    pub desktop: String,
    pub session: String,
    pub gtk: String,
    pub adw: String,
    pub language: String,
    pub locale: String,
    pub scale: i32,
    pub startup_ms: Option<u64>,
}

impl Environment {
    /// The environment of this run.
    pub fn detect() -> Environment {
        use relm4::gtk::prelude::*;
        use relm4::{adw, gtk};
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let scale = gtk::gdk::Display::default()
            .and_then(|d| d.monitors().item(0))
            .and_then(|m: relm4::gtk::glib::Object| m.downcast::<gtk::gdk::Monitor>().ok())
            .map(|m: gtk::gdk::Monitor| m.scale_factor())
            .unwrap_or(1);
        let session = {
            let t = var("XDG_SESSION_TYPE");
            if !t.is_empty() {
                t
            } else if !var("WAYLAND_DISPLAY").is_empty() {
                "wayland".to_string()
            } else if !var("DISPLAY").is_empty() {
                "x11".to_string()
            } else {
                String::new()
            }
        };
        Environment {
            version: crate::beta::version(),
            channel: crate::config::CHANNEL.to_string(),
            commit: crate::config::COMMIT.to_string(),
            profile: crate::config::PROFILE.to_string(),
            packaging: packaging_text(&Packaging::detect()).to_string(),
            os: pretty_name(&os).unwrap_or_else(|| std::env::consts::OS.to_string()),
            kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .unwrap_or_default()
                .trim()
                .to_string(),
            desktop: var("XDG_CURRENT_DESKTOP"),
            session,
            gtk: format!(
                "{}.{}.{}",
                gtk::major_version(),
                gtk::minor_version(),
                gtk::micro_version()
            ),
            adw: format!(
                "{}.{}.{}",
                adw::major_version(),
                adw::minor_version(),
                adw::micro_version()
            ),
            language: crate::i18n::current(),
            locale: var("LANG"),
            scale,
            startup_ms: crate::applog::startup_ms(),
        }
    }
}

/// Switches and counts of the settings: booleans, enums and numbers, never a value the user typed.
#[derive(Debug, Clone, Default)]
pub struct SettingsSummary {
    pub offline: bool,
    pub sync_server: bool,
    pub safe_mode: bool,
    pub eink: bool,
    pub reviewed_locales_only: bool,
    pub plugin_ids: Vec<String>,
    pub patches: usize,
    /// One entry per backup place: its kind (`folder`, `webdav`), never its address.
    pub backup_kinds: Vec<String>,
    /// `system keyring`, `encrypted store`, `not saved`: where passwords are kept.
    pub keyring: String,
    pub tts_engine: String,
}

/// Counts about the library.
#[derive(Debug, Clone, Default)]
pub struct LibraryStats {
    pub books: usize,
    /// Format -> number of files.
    pub formats: BTreeMap<String, usize>,
    pub schema: Option<i32>,
    /// Books whose text is searchable, and the books in total.
    pub indexed: Option<(usize, usize)>,
}

/// One finished or running job.
#[derive(Debug, Clone, Default)]
pub struct JobLine {
    pub kind: String,
    /// `done`, `running`, `cancelled` or `failed: <reason>`.
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub env: Environment,
    pub settings: SettingsSummary,
    pub library: Option<LibraryStats>,
    pub jobs: Vec<JobLine>,
    pub log: Vec<String>,
    pub crash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub include_log: bool,
    pub include_library: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            include_log: true,
            include_library: true,
        }
    }
}

fn onoff(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut c = max;
    while !s.is_char_boundary(c) {
        c -= 1;
    }
    format!("{}…", &s[..c])
}

/// The fixed first lines of an issue's "What happened" part.
pub const WHAT_HAPPENED: &str = "### What happened";

/// The report: the facts (cleaned), then `### What happened` and the user's text (unchanged).
pub fn build(input: &Input, opts: Options, user_text: &str, scrub: &Scrubber) -> String {
    let user = {
        let t = user_text.trim();
        if t.len() > MAX_USER_BYTES {
            format!(
                "{}\n\n_(cut to {} KB)_",
                cut(t, MAX_USER_BYTES),
                MAX_USER_BYTES / 1024
            )
        } else {
            t.to_string()
        }
    };
    let tail = if user.is_empty() {
        format!("\n{WHAT_HAPPENED}\n\n")
    } else {
        format!("\n{WHAT_HAPPENED}\n\n{user}\n")
    };
    let budget = MAX_BYTES.saturating_sub(tail.len());

    // clean the free text once; the structure around it is fixed
    let clean = |s: &str| scrub.apply(s);
    let log: Vec<String> = if opts.include_log {
        let skip = input.log.len().saturating_sub(LOG_LINES);
        input.log[skip..].iter().map(|l| clean(l)).collect()
    } else {
        Vec::new()
    };
    let jobs: Vec<JobLine> = input
        .jobs
        .iter()
        .take(JOB_LINES)
        .map(|j| JobLine {
            kind: j.kind.clone(),
            status: clean(&cut(&j.status, JOB_ERROR_BYTES)),
        })
        .collect();
    let crash = input.crash.as_deref().map(clean);
    let env_text = clean(&facts(input, &jobs, opts));

    let mut keep = log.len();
    let mut dropped = false;
    loop {
        let body = assemble(
            &env_text,
            &log[log.len() - keep..],
            crash.as_deref(),
            opts,
            dropped,
        );
        if body.len() <= budget || keep == 0 {
            let mut out = if body.len() <= budget {
                body
            } else {
                // even without the log: cut the crash text, then the whole
                let mut b = cut(&body, budget.saturating_sub(60));
                b.push_str("\n\n_(report cut to fit 60 KB)_\n");
                b
            };
            out.push_str(&tail);
            return out;
        }
        keep = keep.saturating_sub(keep.div_ceil(4).max(1));
        dropped = true;
    }
}

fn facts(i: &Input, jobs: &[JobLine], opts: Options) -> String {
    let e = &i.env;
    let s = &i.settings;
    let mut t = String::new();
    t.push_str("### Bookx\n\n");
    t.push_str(&format!(
        "- Version: {} ({})\n",
        e.version,
        if e.channel == "stable" {
            "stable".to_string()
        } else {
            format!("{} release", e.channel)
        }
    ));
    if !e.commit.is_empty() {
        t.push_str(&format!("- Commit: {}\n", e.commit));
    }
    t.push_str(&format!("- Installed as: {}\n", e.packaging));
    t.push_str(&format!("- Build: {}\n", e.profile));
    t.push_str("\n### System\n\n");
    t.push_str(&format!("- OS: {}\n", e.os));
    t.push_str(&format!("- Kernel: {}\n", e.kernel));
    t.push_str(&format!("- Desktop: {} ({})\n", e.desktop, e.session));
    t.push_str(&format!("- GTK {}, libadwaita {}\n", e.gtk, e.adw));
    t.push_str(&format!(
        "- Language: {} (locale {}), display scale {}\n",
        e.language, e.locale, e.scale
    ));
    if let Some(ms) = e.startup_ms {
        t.push_str(&format!("- Start-up: {ms} ms\n"));
    }
    t.push_str("\n### Settings\n\n");
    t.push_str(&format!("- Offline mode: {}\n", onoff(s.offline)));
    t.push_str(&format!("- Sync server: {}\n", onoff(s.sync_server)));
    t.push_str(&format!("- Safe mode: {}\n", onoff(s.safe_mode)));
    t.push_str(&format!("- E-ink mode: {}\n", onoff(s.eink)));
    t.push_str(&format!(
        "- Translations: {}\n",
        if s.reviewed_locales_only {
            "reviewed languages only"
        } else {
            "all languages"
        }
    ));
    if s.plugin_ids.is_empty() {
        t.push_str("- Plugins: none\n");
    } else {
        t.push_str(&format!(
            "- Plugins: {} ({})\n",
            s.plugin_ids.len(),
            s.plugin_ids.join(", ")
        ));
    }
    t.push_str(&format!("- Patches: {}\n", s.patches));
    if s.backup_kinds.is_empty() {
        t.push_str("- Backup places: none\n");
    } else {
        t.push_str(&format!(
            "- Backup places: {} ({})\n",
            s.backup_kinds.len(),
            s.backup_kinds.join(", ")
        ));
    }
    t.push_str(&format!("- Passwords kept in: {}\n", s.keyring));
    t.push_str(&format!("- Read aloud engine: {}\n", s.tts_engine));
    if opts.include_library {
        if let Some(l) = &i.library {
            t.push_str("\n### Library\n\n");
            t.push_str(&format!("- Books: {}\n", l.books));
            let mut f: Vec<(&String, &usize)> = l.formats.iter().collect();
            f.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let f: Vec<String> = f.iter().map(|(k, n)| format!("{k} {n}")).collect();
            t.push_str(&format!(
                "- Formats: {}\n",
                if f.is_empty() {
                    "none".into()
                } else {
                    f.join(", ")
                }
            ));
            if let Some(v) = l.schema {
                t.push_str(&format!("- Catalogue schema: {v}\n"));
            }
            match l.indexed {
                Some((n, total)) => t.push_str(&format!("- Search index: {n} of {total} books\n")),
                None => t.push_str("- Search index: not available\n"),
            }
        }
    }
    t.push_str("\n### Recent jobs\n\n");
    if jobs.is_empty() {
        t.push_str("- none\n");
    }
    for j in jobs {
        t.push_str(&format!("- {}: {}\n", j.kind, j.status));
    }
    t
}

fn assemble(
    facts: &str,
    log: &[String],
    crash: Option<&str>,
    opts: Options,
    dropped: bool,
) -> String {
    let mut s =
        String::from("<!-- Bookx problem report, made on this computer. Nothing was sent. -->\n\n");
    s.push_str(facts);
    if opts.include_log {
        s.push_str("\n### Recent log\n\n");
        if dropped {
            s.push_str("_(older lines left out to fit 60 KB)_\n\n");
        }
        s.push_str("```text\n");
        if log.is_empty() {
            s.push_str("(nothing logged)\n");
        }
        for l in log {
            s.push_str(l);
            s.push('\n');
        }
        s.push_str("```\n");
    }
    if let Some(c) = crash {
        s.push_str("\n### Last crash\n\n```text\n");
        s.push_str(c.trim_end());
        s.push_str("\n```\n");
    }
    s
}

/// The settings summary of the running app.
pub fn summarize(
    cfg: &crate::config::Config,
    mgr: &bookx_plugin::PluginManager,
) -> SettingsSummary {
    SettingsSummary {
        offline: cfg.offline,
        sync_server: cfg.share.running || cfg.share.lan || cfg.share.kosync,
        safe_mode: crate::safe_mode::active(),
        eink: cfg.eink,
        reviewed_locales_only: crate::i18n::REVIEWED_ONLY,
        plugin_ids: mgr
            .list()
            .into_iter()
            .filter(|p| !p.id.is_empty())
            .map(|p| p.id)
            .collect(),
        patches: std::fs::read_dir(crate::plugins::patches_dir())
            .map(|rd| rd.flatten().count())
            .unwrap_or(0),
        backup_kinds: cfg
            .health
            .backup
            .destinations
            .iter()
            .map(|d| format!("{:?}", d.kind).to_ascii_lowercase())
            .collect(),
        keyring: match crate::secrets::vault().mode() {
            bookx_secrets::Mode::Keyring => "system keyring",
            bookx_secrets::Mode::Vault => "encrypted store",
            _ => "not saved (session only)",
        }
        .to_string(),
        tts_engine: cfg.tts.engine.clone(),
    }
}

/// Counts about the library (no titles, no paths).
pub fn library_stats(
    lib: &crate::core::library::Library,
    indexed: Option<(usize, usize)>,
) -> LibraryStats {
    let mut formats = BTreeMap::new();
    for b in &lib.books {
        for f in &b.formats {
            *formats.entry(f.format.to_ascii_lowercase()).or_insert(0) += 1;
        }
    }
    LibraryStats {
        books: lib.books.len(),
        formats,
        schema: Some(bookx_store::SCHEMA_VERSION),
        indexed,
    }
}

/// The running jobs and the last finished ones, newest first.
pub fn recent_jobs() -> Vec<JobLine> {
    crate::jobs::with(|h| {
        h.jobs()
            .iter()
            .rev()
            .take(JOB_LINES)
            .map(|j| JobLine {
                kind: j.kind.label().to_string(),
                status: match &j.state {
                    crate::jobs::State::Running => "running".into(),
                    crate::jobs::State::Done => "done".into(),
                    crate::jobs::State::Cancelled => "cancelled".into(),
                    crate::jobs::State::Failed(e) => format!("failed: {e}"),
                },
            })
            .collect()
    })
}

/// Writes the report to `path`, readable by its owner only (it describes the user's machine).
pub fn write_report(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // a file the chooser created before has its own permissions
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(text.as_bytes())
}

/// The URL of the bug form; only a short title is added, the report goes through the clipboard.
pub const ISSUE_URL: &str =
    "https://github.com/adhadse/Bookx/issues/new?template=bug_report.yml&title=Bug%3A%20";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug_log::Scrub;

    fn env() -> Environment {
        Environment {
            version: "0.3.0-beta.1".into(),
            channel: "beta".into(),
            commit: "abc123def".into(),
            profile: "release".into(),
            packaging: "Flatpak".into(),
            os: "Fedora Linux 43 (Workstation Edition)".into(),
            kernel: "6.17.7-300.fc43.x86_64".into(),
            desktop: "GNOME".into(),
            session: "wayland".into(),
            gtk: "4.20.1".into(),
            adw: "1.8.0".into(),
            language: "en".into(),
            locale: "en_US.UTF-8".into(),
            scale: 2,
            startup_ms: Some(840),
        }
    }

    fn input() -> Input {
        Input {
            env: env(),
            settings: SettingsSummary {
                offline: false,
                sync_server: true,
                safe_mode: false,
                eink: false,
                reviewed_locales_only: true,
                plugin_ids: vec!["acme.trim".into()],
                patches: 1,
                backup_kinds: vec!["folder".into(), "webdav".into()],
                keyring: "system keyring".into(),
                tts_engine: "piper".into(),
            },
            library: Some(LibraryStats {
                books: 12,
                formats: [("epub".to_string(), 10), ("pdf".to_string(), 2)]
                    .into_iter()
                    .collect(),
                schema: Some(9),
                indexed: Some((10, 12)),
            }),
            jobs: vec![
                JobLine {
                    kind: "Import".into(),
                    status: "done".into(),
                },
                JobLine {
                    kind: "Backup".into(),
                    status: "failed: cannot reach sync.example.org".into(),
                },
            ],
            log: vec![
                "2026-11-22T10:00:00Z  INFO bookx_crumb: start Bookx 0.3.0-beta.1".into(),
                "2026-11-22T10:00:05Z  WARN bookx: plugin acme.trim could not be loaded".into(),
            ],
            crash: None,
        }
    }

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

    const GOLDEN: &str = "<!-- Bookx problem report, made on this computer. Nothing was sent. -->

### Bookx

- Version: 0.3.0-beta.1 (beta release)
- Commit: abc123def
- Installed as: Flatpak
- Build: release

### System

- OS: Fedora Linux 43 (Workstation Edition)
- Kernel: 6.17.7-300.fc43.x86_64
- Desktop: GNOME (wayland)
- GTK 4.20.1, libadwaita 1.8.0
- Language: en (locale en_US.UTF-8), display scale 2
- Start-up: 840 ms

### Settings

- Offline mode: off
- Sync server: on
- Safe mode: off
- E-ink mode: off
- Translations: reviewed languages only
- Plugins: 1 (acme.trim)
- Patches: 1
- Backup places: 2 (folder, webdav)
- Passwords kept in: system keyring
- Read aloud engine: piper

### Library

- Books: 12
- Formats: epub 10, pdf 2
- Catalogue schema: 9
- Search index: 10 of 12 books

### Recent jobs

- Import: done
- Backup: failed: cannot reach <host>

### Recent log

```text
2026-11-22T10:00:00Z  INFO bookx_crumb: start Bookx 0.3.0-beta.1
2026-11-22T10:00:05Z  WARN bookx: plugin acme.trim could not be loaded
```

### What happened

";

    #[test]
    fn the_report_matches_the_golden_text() {
        let r = build(&input(), Options::default(), "", &scrub());
        assert_eq!(r, GOLDEN);
        // deterministic
        assert_eq!(r, build(&input(), Options::default(), "", &scrub()));
    }

    #[test]
    fn the_user_text_is_added_unchanged_after_the_facts() {
        let r = build(
            &input(),
            Options::default(),
            "  It crashed when I opened alice's book  \n",
            &scrub(),
        );
        assert!(r.ends_with("### What happened\n\nIt crashed when I opened alice's book\n"));
    }

    #[test]
    fn the_toggles_leave_out_the_log_and_the_library() {
        let r = build(
            &input(),
            Options {
                include_log: false,
                include_library: false,
            },
            "",
            &scrub(),
        );
        assert!(!r.contains("### Recent log") && !r.contains("### Library"));
        assert!(r.contains("### Recent jobs") && r.contains("### What happened"));
    }

    #[test]
    fn no_sentinel_gets_through() {
        let mut i = input();
        i.log = vec![
            "ERROR open The Secret Garden at /home/alice/Books/x.epub as alice_reads".into(),
            "WARN login hunter2-pass https://bob:pw123@sync.example.org/x?token=abcdef12 from 203.0.113.9 and 192.168.1.7".into(),
            "WARN Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig mail alice@example.org".into(),
            "WARN key 0123456789abcdef0123456789abcdef01234567 b64 QWxhZGRpbjpvcGVuIHNlc2FtZQ==".into(),
            "WARN v6 2001:db8::8a2e:370:7334 and fd00::1".into(),
        ];
        i.crash = Some("panic in /home/alice/src/x.rs for The Secret Garden".into());
        i.jobs[0].status = "failed: /home/alice/Books/The Secret Garden.epub".into();
        let r = build(&i, Options::default(), "", &scrub());
        for bad in [
            "Secret Garden",
            "alice",
            "hunter2",
            "sync.example.org",
            "bob",
            "pw123",
            "abcdef12",
            "203.0.113.9",
            "192.168.1.7",
            "eyJhbGci",
            "0123456789abcdef",
            "QWxhZGRpbjpvcGVu",
            "2001:db8",
            "fd00::1",
        ] {
            assert!(!r.contains(bad), "{bad} leaked:\n{r}");
        }
        assert!(r.contains("<lan-ip>") && r.contains("<ip>") && r.contains("<title>"));
        assert!(r.contains("### Last crash"));
    }

    #[test]
    fn a_long_log_is_cut_to_the_cap_with_a_notice() {
        let mut i = input();
        i.log = (0..LOG_LINES * 3)
            .map(|n| format!("2026 WARN line {n} {}", "word ".repeat(80)))
            .collect();
        i.crash = Some("crash line\n".repeat(3_000));
        let r = build(&i, Options::default(), "my words", &scrub());
        assert!(r.len() <= MAX_BYTES, "{}", r.len());
        assert!(r.contains("older lines left out to fit 60 KB"));
        assert!(r.ends_with("### What happened\n\nmy words\n"));
        // the newest lines are the ones kept
        assert!(r.contains(&format!("line {} ", LOG_LINES * 3 - 1)));
    }

    #[test]
    fn at_most_the_last_200_lines_are_used() {
        let mut i = input();
        i.log = (0..500).map(|n| format!("WARN n{n:03}")).collect();
        let r = build(&i, Options::default(), "", &scrub());
        assert!(r.contains("n499") && r.contains("n300") && !r.contains("n299"));
    }

    #[test]
    fn a_huge_user_text_is_cut() {
        let r = build(&input(), Options::default(), &"y".repeat(20_000), &scrub());
        assert!(r.len() <= MAX_BYTES && r.contains("_(cut to 8 KB)_"));
    }

    #[test]
    fn packaging_is_told_from_the_environment() {
        let p = |f: bool, exe: &str, ids: &str| {
            Packaging::detect_from(&PackagingProbe {
                flatpak_id: f,
                exe: exe.into(),
                os_ids: ids.into(),
                ..PackagingProbe::default()
            })
        };
        assert_eq!(p(true, "/app/bin/bookx", ""), Packaging::Flatpak);
        assert_eq!(p(false, "/app/bin/bookx", ""), Packaging::Flatpak);
        assert_eq!(p(false, "/usr/bin/bookx", "ubuntu debian"), Packaging::Deb);
        assert_eq!(p(false, "/usr/bin/bookx", "fedora "), Packaging::Rpm);
        assert_eq!(
            p(false, "/usr/bin/bookx", "arch "),
            Packaging::SystemPackage
        );
        assert_eq!(
            p(false, "/home/a/bookx/bookx", "fedora"),
            Packaging::Tarball
        );
        assert_eq!(
            Packaging::detect_from(&PackagingProbe {
                appimage: true,
                exe: "/tmp/.mount_x/usr/bin/bookx".into(),
                ..PackagingProbe::default()
            }),
            Packaging::AppImage
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_saved_report_is_private_even_over_an_open_file() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!("bookx-report-{}.md", std::process::id()));
        std::fs::write(&p, "old and longer than the new text").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_report(&p, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn the_issue_address_is_safe_and_carries_only_a_short_title() {
        assert!(bookx_core::util::safe_external_url(ISSUE_URL).is_some());
        assert!(ISSUE_URL
            .starts_with("https://github.com/adhadse/Bookx/issues/new?template=bug_report.yml"));
        assert!(ISSUE_URL.len() < 120);
    }

    #[test]
    fn os_release_is_read() {
        let t = "NAME=\"Fedora Linux\"\nID=fedora\nID_LIKE=\"rhel centos\"\nPRETTY_NAME=\"Fedora Linux 43\"\n";
        assert_eq!(pretty_name(t).as_deref(), Some("Fedora Linux 43"));
        assert_eq!(os_ids(t).trim(), "fedora rhel centos");
        assert_eq!(pretty_name("X=1"), None);
    }
}
