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
use crate::diag_facts::{Redactor, Section};
use crate::diag_problems;
use crate::trace;
use crate::triage;

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
        let scale = gtk::gdk::Display::default()
            .and_then(|d| d.monitors().item(0))
            .and_then(|m: relm4::gtk::glib::Object| m.downcast::<gtk::gdk::Monitor>().ok())
            .map(|m: gtk::gdk::Monitor| m.scale_factor())
            .unwrap_or(1);
        let versions = (
            format!(
                "{}.{}.{}",
                gtk::major_version(),
                gtk::minor_version(),
                gtk::micro_version()
            ),
            format!(
                "{}.{}.{}",
                adw::major_version(),
                adw::minor_version(),
                adw::micro_version()
            ),
        );
        Environment::detect_with(|k| std::env::var(k).unwrap_or_default(), scale, versions)
    }

    /// [`Environment::detect`] with the environment variables, the display scale and the GTK and
    /// libadwaita versions given (asking for them needs an initialised toolkit).
    pub fn detect_with(
        var: impl Fn(&str) -> String,
        scale: i32,
        (gtk, adw): (String, String),
    ) -> Environment {
        let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
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
            gtk,
            adw,
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
    /// The state of the features (the machine, the package, the library folder, every registered
    /// snapshot provider, the books involved), already as facts: the only way into the report.
    pub sections: Vec<Section>,
    /// What happened in this run, and before it (from the log), oldest first.
    pub trace: Vec<trace::Entry>,
    /// The tail the last session left when it was closed or crashed.
    pub previous: Vec<trace::Entry>,
    /// The area the user pressed Report in ("Sync window").
    pub entry: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub include_log: bool,
    pub include_library: bool,
    /// Name the books involved by title and author (default off).
    pub include_titles: bool,
    /// Show the host of server addresses (default off).
    pub include_host: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            include_log: true,
            include_library: true,
            include_titles: false,
            include_host: false,
        }
    }
}

fn onoff(b: bool) -> &'static str {
    if b { "on" } else { "off" }
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

/// Room kept for the summary block.
const SUMMARY_RESERVE: usize = 2200;
/// Operations listed in "Recent operations".
const OPERATIONS: usize = 40;

/// The report: the summary, what happened, the facts (cleaned), then `### What happened` and the user's
/// text (unchanged).
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
    let budget = MAX_BYTES.saturating_sub(tail.len() + SUMMARY_RESERVE);

    // free text is cleaned once; the structure around it is fixed, and every fact of a feature goes
    // through the redactor
    let clean = |s: &str| scrub.apply(s);
    let red = Redactor {
        scrub,
        show_host: opts.include_host,
        show_titles: opts.include_titles,
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

    let mut top = String::new();
    top.push_str(&operations_text(&input.trace, &red));
    let log_all: &[String] = if opts.include_log { &input.log } else { &[] };
    top.push_str(&problems_text(log_all, &red));
    top.push_str(&previous_text(&input.previous, &red));
    // the area the user pressed Report in comes first
    let mut sections: Vec<&Section> = input.sections.iter().collect();
    if let Some(e) = &input.entry {
        sections.sort_by_key(|s| s.title != *e);
    }
    for sec in sections {
        let t = red.section(sec);
        if !t.is_empty() {
            top.push_str(&t);
            top.push('\n');
        }
    }
    let env_text = clean(&facts(input, &jobs, opts));

    let fixed = top.len() + env_text.len() + crash.as_ref().map_or(0, |c| c.len() + 40) + 200;
    let log_budget = budget.saturating_sub(fixed);
    let plan = diag_problems::fit_log(
        &input.log[input.log.len().saturating_sub(LOG_LINES * 4)..],
        LOG_LINES,
        log_budget,
    );
    let log: Vec<String> = if opts.include_log {
        plan.lines.iter().map(|l| clean_log_line(l, scrub)).collect()
    } else {
        Vec::new()
    };
    let body = assemble(&top, &env_text, &log, crash.as_deref(), opts, &plan);
    let body = if body.len() > budget {
        let mut b = cut(&body, budget.saturating_sub(60));
        b.push_str("\n\n_(report cut to fit 60 KB)_\n");
        b
    } else {
        body
    };

    // the summary reads the cleaned text, so it can never show more than the report does
    let findings = triage::analyse(&body);
    let mut summary = triage::summary_block(&findings, input.entry.as_deref());
    if summary.len() > SUMMARY_RESERVE {
        summary = cut(&summary, SUMMARY_RESERVE - 4);
        summary.push_str("\n");
    }
    let mut out = String::from("<!-- Bookx problem report, made on this computer. Nothing was sent. -->\n\n");
    out.push_str(&summary);
    out.push('\n');
    out.push_str(body.strip_prefix(HEADER).unwrap_or(&body));
    out.push_str(&tail);
    out
}

const HEADER: &str = "<!-- Bookx problem report, made on this computer. Nothing was sent. -->\n\n";

/// A log line as shown: the scrubber for everything except the fields of a `trace` breadcrumb, which
/// keep their hash (see [`crate::applog::clean_line`]).
fn clean_log_line(l: &str, scrub: &Scrubber) -> String {
    crate::applog::clean_line(l, scrub)
}

fn time_of(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M:%SZ").to_string())
        .unwrap_or_default()
}

/// `### Recent operations`: failures first (the ones nothing healed), then the newest of every area.
fn operations_text(entries: &[trace::Entry], red: &Redactor) -> String {
    if entries.is_empty() {
        return "### Recent operations\n\n- none recorded yet in this run\n\n".to_string();
    }
    let line = |e: &trace::Entry| red_line(e, red);
    let mut s = String::from("### Recent operations\n\n");
    if let Some(f) = trace::last_failure(entries) {
        s.push_str(&format!("- Last failure: {}\n", line(f)));
    }
    let skip = entries.len().saturating_sub(OPERATIONS);
    for e in &entries[skip..] {
        s.push_str(&format!("- {}\n", line(e)));
    }
    s.push('\n');
    s
}

fn red_line(e: &trace::Entry, red: &Redactor) -> String {
    e.summary_with(&|m| red.text(m))
}

fn previous_text(entries: &[trace::Entry], red: &Redactor) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut s = String::from("### Last session (before this start)\n\n");
    for e in entries.iter().rev().take(20).collect::<Vec<_>>().into_iter().rev() {
        s.push_str(&format!("- {} {}\n", time_of(e.time), red_line(e, red)));
    }
    s.push('\n');
    s
}

/// `### Recent problems`: one row per kind of warning or error, with a count, harmless noise left out.
fn problems_text(log: &[String], red: &Redactor) -> String {
    let a = diag_problems::analyse(log);
    if a.problems.is_empty() && a.noise.is_empty() {
        return String::new();
    }
    let mut s = String::from("### Recent problems\n\n");
    if a.problems.is_empty() {
        s.push_str("- none (only harmless notices)\n");
    }
    for p in &a.problems {
        let lvl = match p.level {
            diag_problems::Level::Error => "ERROR",
            diag_problems::Level::Warn => "WARN",
        };
        let n = if p.count > 1 {
            format!(" ×{}", p.count)
        } else {
            String::new()
        };
        let last = p.last.get(11..19).unwrap_or("");
        let target = if p.target.is_empty() { String::new() } else { format!("{}: ", p.target) };
        s.push_str(&format!(
            "- {lvl}{n} (last {last}Z) {target}{}\n",
            red.text(&p.message)
        ));
    }
    for (name, n) in &a.noise {
        s.push_str(&format!("- left out as harmless: {name} ({n} lines)\n"));
    }
    s.push('\n');
    s
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
    top: &str,
    facts: &str,
    log: &[String],
    crash: Option<&str>,
    opts: Options,
    plan: &diag_problems::LogPlan,
) -> String {
    let mut s = String::from(HEADER);
    s.push_str(top);
    s.push_str(facts);
    if opts.include_log {
        s.push_str("\n### Recent log\n\n");
        let mut notes = Vec::new();
        if plan.noise > 0 {
            notes.push(format!("{} harmless lines left out", plan.noise));
        }
        if plan.folded > 0 {
            notes.push(format!("{} repeats folded into [×N]", plan.folded));
        }
        if plan.dropped > 0 {
            notes.push(format!(
                "{} less important lines left out to fit 60 KB (errors and breadcrumbs are kept first)",
                plan.dropped
            ));
        }
        if !notes.is_empty() {
            s.push_str(&format!("_({})_\n\n", notes.join("; ")));
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
    summarize_with(
        cfg,
        mgr.list()
            .into_iter()
            .filter(|p| !p.id.is_empty())
            .map(|p| p.id)
            .collect(),
        std::fs::read_dir(crate::plugins::patches_dir())
            .map(|rd| rd.flatten().count())
            .unwrap_or(0),
        match crate::secrets::vault().mode() {
            bookx_secrets::Mode::Keyring => "system keyring",
            bookx_secrets::Mode::Vault => "encrypted store",
            bookx_secrets::Mode::Locked => "encrypted store (locked)",
            _ => "not saved (session only)",
        },
    )
}

/// [`summarize`] from the parts it reads from the running app.
pub fn summarize_with(
    cfg: &crate::config::Config,
    plugin_ids: Vec<String>,
    patches: usize,
    keyring: &str,
) -> SettingsSummary {
    SettingsSummary {
        offline: cfg.offline,
        sync_server: cfg.share.running || cfg.share.lan || cfg.share.kosync,
        safe_mode: crate::safe_mode::active(),
        eink: cfg.eink,
        reviewed_locales_only: crate::i18n::REVIEWED_ONLY,
        plugin_ids,
        patches,
        backup_kinds: cfg
            .health
            .backup
            .destinations
            .iter()
            .map(|d| format!("{:?}", d.kind).to_ascii_lowercase())
            .collect(),
        keyring: keyring.to_string(),
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
            ..Default::default()
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

### Summary and suspected area

- Suspected area: Plugins and patches
- Severity: annoyance
- What went wrong: A plugin failed to load or run: acme.trim.
- Hint: Disable the plugin (or start in safe mode) to confirm; report to the plugin author with its version.
- Evidence: `- WARN (last 10:00:05Z) bookx: plugin acme.trim could not be loaded`
- Evidence: `Plugins: 1 (acme.trim)`
- Also noticed: Unknown (A problem was logged; see Recent problems)

### Recent operations

- none recorded yet in this run

### Recent problems

- WARN (last 10:00:05Z) bookx: plugin acme.trim could not be loaded

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
                ..Options::default()
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
        let name = |n: usize| -> String {
            let mut n = n;
            let mut s = String::new();
            loop {
                s.push((b'a' + (n % 26) as u8) as char);
                n /= 26;
                if n == 0 {
                    return s;
                }
            }
        };
        i.log = (0..LOG_LINES * 3)
            .map(|n| format!("2026 WARN bookx: line {} {}", name(n), "word ".repeat(80)))
            .collect();
        i.crash = Some("crash line\n".repeat(3_000));
        let r = build(&i, Options::default(), "my words", &scrub());
        assert!(r.len() <= MAX_BYTES, "{}", r.len());
        assert!(r.contains("left out to fit 60 KB"));
        assert!(r.ends_with("### What happened\n\nmy words\n"));
        // the newest lines are the ones kept
        assert!(r.contains(&format!("line {} ", name(LOG_LINES * 3 - 1))));
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
        assert!(
            ISSUE_URL
                .starts_with("https://github.com/adhadse/Bookx/issues/new?template=bug_report.yml")
        );
        assert!(ISSUE_URL.len() < 120);
    }

    #[test]
    fn os_release_is_read() {
        let t = "NAME=\"Fedora Linux\"\nID=fedora\nID_LIKE=\"rhel centos\"\nPRETTY_NAME=\"Fedora Linux 43\"\n";
        assert_eq!(pretty_name(t).as_deref(), Some("Fedora Linux 43"));
        assert_eq!(os_ids(t).trim(), "fedora rhel centos");
        assert_eq!(pretty_name("X=1"), None);
    }

    #[test]
    fn the_environment_is_read_from_the_given_variables() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            }
        };
        let e = Environment::detect_with(
            vars(&[
                ("XDG_SESSION_TYPE", "tty"),
                ("XDG_CURRENT_DESKTOP", "KDE"),
                ("LANG", "de_DE.UTF-8"),
            ]),
            2,
            ("4.20.1".into(), "1.8.0".into()),
        );
        assert_eq!((e.session.as_str(), e.desktop.as_str()), ("tty", "KDE"));
        assert_eq!((e.locale.as_str(), e.scale), ("de_DE.UTF-8", 2));
        assert_eq!((e.gtk.as_str(), e.adw.as_str()), ("4.20.1", "1.8.0"));
        assert!(!e.version.is_empty() && !e.os.is_empty() && !e.packaging.is_empty());
        let s = |p| Environment::detect_with(vars(p), 1, Default::default()).session;
        assert_eq!(
            s(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]),
            "wayland"
        );
        assert_eq!(s(&[("DISPLAY", ":0")]), "x11");
        assert_eq!(s(&[]), "");
        // does not need a display or an environment
        let _ = Packaging::detect();
    }

    #[test]
    fn packaging_names_are_all_distinct() {
        let all = [
            Packaging::Flatpak,
            Packaging::AppImage,
            Packaging::Deb,
            Packaging::Rpm,
            Packaging::SystemPackage,
            Packaging::Tarball,
        ];
        let names: std::collections::BTreeSet<_> = all.iter().map(packaging_text).collect();
        assert_eq!(names.len(), all.len());
    }

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-report-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_settings_summary_holds_switches_and_no_values() {
        let d = tmpdir("settings");
        let mgr = bookx_plugin::PluginManager::new(
            d.join("plugins"),
            d.join("state.json"),
            bookx_plugin::Host::default(),
        );
        let mut cfg = crate::config::Config {
            offline: true,
            eink: true,
            ..Default::default()
        };
        cfg.share.kosync = true;
        cfg.tts.engine = "piper".into();
        let s = summarize(&cfg, &mgr);
        assert!(s.offline && s.eink && s.sync_server);
        assert_eq!(s.tts_engine, "piper");
        assert!(s.plugin_ids.is_empty());
        assert!(!s.keyring.is_empty());
        let s = summarize(&crate::config::Config::default(), &mgr);
        assert!(!s.offline && !s.eink);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_library_stats_count_books_without_titles() {
        let d = tmpdir("stats");
        let mut lib = crate::core::library::Library::in_memory(&d.join("lib")).unwrap();
        for t in ["One", "Two"] {
            let p = d.join(format!("{t}.md"));
            std::fs::write(&p, format!("---\ntitle: \"{t}\"\n---\n# A\n\nHi.\n")).unwrap();
            lib.add_path(&p, true).unwrap();
        }
        let st = library_stats(&lib, Some((1, 2)));
        assert_eq!(st.books, 2);
        assert_eq!(st.indexed, Some((1, 2)));
        assert_eq!(st.schema, Some(bookx_store::SCHEMA_VERSION));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn recent_jobs_are_listed_newest_first_with_their_state() {
        use crate::jobs;
        // the job list is per thread, so this test sees only its own jobs
        let (a, _) = jobs::start(jobs::Kind::Import, "i", false);
        let (b, _) = jobs::start(jobs::Kind::Backup, "b", true);
        let (c, _) = jobs::start(jobs::Kind::Export, "e", true);
        let (_d, _) = jobs::start(jobs::Kind::Fetch, "f", true);
        jobs::finish(a, Ok(()));
        jobs::finish(b, Err("no route".into()));
        assert!(jobs::cancel(c));
        jobs::finish(c, Ok(()));
        let v = recent_jobs();
        let status: Vec<(&str, &str)> = v
            .iter()
            .map(|j| (j.kind.as_str(), j.status.as_str()))
            .collect();
        assert_eq!(
            status,
            [
                ("Metadata", "running"),
                ("Export", "cancelled"),
                ("Backup", "failed: no route"),
                ("Import", "done"),
            ]
        );
    }
}
