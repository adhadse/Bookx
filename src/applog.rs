//! The persistent local log and the crash files of "Report a Problem".
//!
//! Bookx keeps a small rotating log next to its other state (`$XDG_STATE_HOME/bookx/bookx.log`, at most two
//! files of 1 MB: `bookx.log` and `bookx.log.1`; the folder is `0700`, the files `0600`). It holds the
//! warnings and errors and a few breadcrumbs (`tracing::info!(target: "bookx_crumb", ...)`: start, library
//! opened with counts, server start and stop, backup results, plugin load results, failed imports and
//! conversions). Every line is cleaned by the [`Scrubber`] before it is written (paths below the home folder,
//! the library folder, the user's servers and accounts, secrets, titles, addresses), and callers do not put
//! titles or secrets into messages in the first place.
//!
//! A panic writes `crash-<UTC time>.txt` beside the log (message, location, backtrace, all cleaned) and a line
//! into the log. On the next start [`unseen_crash`] finds it so the app can offer to report it; Bookx never
//! uploads anything: the user submits the report by hand.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use crate::debug_log::{Scrub, Scrubber};

/// The size at which `bookx.log` is rotated to `bookx.log.1`.
pub const MAX_BYTES: u64 = 1 << 20;
/// Crash files kept (the newest ones).
pub const KEEP_CRASHES: usize = 5;
/// The longest crash file, in bytes.
const MAX_CRASH_BYTES: usize = 48 * 1024;
/// How long a crash file counts as the cause of what a report is about.
pub const CRASH_FRESH: std::time::Duration = std::time::Duration::from_secs(3 * 24 * 3600);
/// The tracing target of the breadcrumbs.
pub const CRUMB: &str = "bookx_crumb";

const LOG: &str = "bookx.log";
const OLD: &str = "bookx.log.1";

/// The files of one log folder.
#[derive(Debug, Clone)]
pub struct LogFile {
    pub dir: PathBuf,
    pub max: u64,
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

fn private_file(path: &Path, append: bool) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.create(true);
    if append {
        o.append(true);
    } else {
        o.write(true).truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

impl LogFile {
    pub fn new(dir: PathBuf) -> LogFile {
        LogFile {
            dir,
            max: MAX_BYTES,
        }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(LOG)
    }

    /// Adds one line; rotates first when it would not fit.
    pub fn append(&self, line: &str) -> std::io::Result<()> {
        private_dir(&self.dir)?;
        let line = line.trim_end_matches(['\n', '\r']);
        let path = self.path();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size > 0 && size + line.len() as u64 + 1 > self.max {
            // the older file is replaced: two files of at most `max` bytes are all there ever is
            let _ = std::fs::rename(&path, self.dir.join(OLD));
        }
        let mut f = private_file(&path, true)?;
        // a very long line is cut so that one line can never exceed the cap
        let mut end = line.len().min(self.max as usize / 4);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        f.write_all(&line.as_bytes()[..end])?;
        f.write_all(b"\n")
    }

    /// The last `n` lines of the older and the current file.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let mut all: Vec<String> = Vec::new();
        for name in [OLD, LOG] {
            if let Ok(t) = std::fs::read_to_string(self.dir.join(name)) {
                all.extend(t.lines().map(str::to_string));
            }
        }
        let skip = all.len().saturating_sub(n);
        all.split_off(skip)
    }

    /// The crash files, newest first (`.txt` unseen, `.txt.seen` after the user dismissed the banner).
    pub fn crashes(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                            n.starts_with("crash-")
                                && (n.ends_with(".txt") || n.ends_with(".txt.seen"))
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v.reverse();
        v
    }

    /// The newest crash file the user has not dismissed.
    pub fn unseen_crash(&self) -> Option<PathBuf> {
        let newest = self.crashes().into_iter().next()?;
        (newest.extension().is_some_and(|e| e == "txt")).then_some(newest)
    }

    /// The text of the newest crash file, seen or not, when it is at most [`CRASH_FRESH`] old: a report
    /// made weeks after a crash is about something else.
    pub fn last_crash_text(&self) -> Option<String> {
        let p = self.crashes().into_iter().next()?;
        let age = std::fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())?;
        (age <= CRASH_FRESH).then(|| std::fs::read_to_string(p).ok())?
    }

    /// Marks every crash file as seen (the banner was dismissed or the report was opened).
    pub fn dismiss_crashes(&self) {
        for p in self.crashes() {
            if p.extension().is_some_and(|e| e == "txt") {
                let mut to = p.clone().into_os_string();
                to.push(".seen");
                let _ = std::fs::rename(&p, PathBuf::from(to));
            }
        }
    }

    fn prune_crashes(&self) {
        for p in self.crashes().into_iter().skip(KEEP_CRASHES) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// What a panic knew, without the hook machinery (the tests build one by hand).
#[derive(Debug, Clone, Default)]
pub struct CrashInfo {
    pub message: String,
    pub location: Option<String>,
    pub thread: String,
    pub backtrace: String,
}

impl CrashInfo {
    pub fn from_hook(info: &std::panic::PanicHookInfo<'_>) -> CrashInfo {
        CrashInfo::from_parts(
            info.payload(),
            info.location().map(|l| (l.file(), l.line(), l.column())),
            std::thread::current().name(),
            std::backtrace::Backtrace::force_capture().to_string(),
        )
    }

    /// The pure part of [`CrashInfo::from_hook`].
    pub fn from_parts(
        payload: &(dyn std::any::Any + Send),
        location: Option<(&str, u32, u32)>,
        thread: Option<&str>,
        backtrace: String,
    ) -> CrashInfo {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic without a message".to_string());
        CrashInfo {
            message,
            location: location.map(|(f, l, c)| format!("{f}:{l}:{c}")),
            thread: thread.unwrap_or("unnamed").to_string(),
            backtrace,
        }
    }
}

/// The text of a crash file. Not yet cleaned: [`write_crash`] runs it through the scrubber.
pub fn crash_text(info: &CrashInfo, when: &str) -> String {
    let mut s = format!(
        "Bookx {} ({}) closed unexpectedly at {when} UTC\nthread: {}\nmessage: {}\n",
        crate::config::VERSION,
        crate::config::CHANNEL,
        info.thread,
        info.message.lines().next().unwrap_or("")
    );
    if let Some(l) = &info.location {
        s.push_str(&format!("at: {l}\n"));
    }
    s.push_str("\nbacktrace:\n");
    // the compiler's commit hash in `/rustc/<hash>/` is noise, and would look like a secret
    let bt = info.backtrace.replace('\r', "");
    let mut out = String::new();
    for l in bt.lines() {
        out.push_str(&strip_rustc_hash(l));
        out.push('\n');
    }
    s.push_str(&out);
    if s.len() > MAX_CRASH_BYTES {
        let mut cut = MAX_CRASH_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n(cut)\n");
    }
    s
}

fn strip_rustc_hash(line: &str) -> String {
    match line.find("/rustc/") {
        Some(i) => {
            let rest = &line[i + 7..];
            let hash_len = rest.chars().take_while(|c| c.is_ascii_hexdigit()).count();
            if hash_len >= 20 {
                format!("{}/rustc/{}", &line[..i], &rest[hash_len..])
            } else {
                line.to_string()
            }
        }
        None => line.to_string(),
    }
}

/// Writes the crash file and a log line; returns the file. The text is cleaned first.
pub fn write_crash(
    log: &LogFile,
    info: &CrashInfo,
    scrubber: &Scrubber,
    now: chrono::DateTime<chrono::Utc>,
) -> std::io::Result<PathBuf> {
    private_dir(&log.dir)?;
    let stamp = now.format("%Y%m%d-%H%M%S").to_string();
    let text = scrubber.apply(&crash_text(
        info,
        &now.format("%Y-%m-%d %H:%M:%S").to_string(),
    ));
    let mut path = log.dir.join(format!("crash-{stamp}.txt"));
    let mut n = 1;
    while path.exists() {
        path = log.dir.join(format!("crash-{stamp}-{n}.txt"));
        n += 1;
    }
    private_file(&path, false)?.write_all(text.as_bytes())?;
    let first = scrubber.apply(&format!(
        "{} ERROR panic: {} at {}",
        now.format("%Y-%m-%dT%H:%M:%SZ"),
        info.message.lines().next().unwrap_or(""),
        info.location.as_deref().unwrap_or("?")
    ));
    let _ = log.append(&first);
    log.prune_crashes();
    Ok(path)
}

// ---- the log of this process ----

static FILE: OnceLock<LogFile> = OnceLock::new();
static SCRUBBER: Mutex<Option<Arc<Scrubber>>> = Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();
static STARTUP_MS: AtomicU64 = AtomicU64::new(0);

/// The folder of the log and the crash files.
pub fn state_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx").map(|d| {
        d.state_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| d.data_local_dir().join("state"))
    })
}

/// The log of this process, once [`init`] has run.
pub fn file() -> Option<&'static LogFile> {
    FILE.get()
}

/// Starts the log and the panic hook. Call once, first thing in `main`.
pub fn init() {
    START.get_or_init(Instant::now);
    if let Some(dir) = state_dir() {
        let _ = FILE.set(LogFile::new(dir));
    }
    set_scrub(Scrub {
        home: home_dir_text(),
        user: std::env::var("USER").unwrap_or_default(),
        ..Scrub::default()
    });
    install_panic_hook();
    breadcrumb_start();
}

fn breadcrumb_start() {
    tracing::info!(
        target: "bookx_crumb",
        "start Bookx {} ({}), {}",
        crate::config::VERSION,
        crate::config::CHANNEL,
        crate::report::packaging_text(&crate::report::Packaging::detect())
    );
}

pub fn home_dir_text() -> String {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().display().to_string())
        .unwrap_or_default()
}

/// Replaces the rules the log is cleaned with (the library folder, titles, servers and secrets are
/// known once the library is open and whenever the settings change).
pub fn set_scrub(rules: Scrub) {
    let s = Arc::new(Scrubber::new(rules));
    *SCRUBBER.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
}

/// The scrubber in use now (home folder and user at least).
pub fn scrubber() -> Arc<Scrubber> {
    let g = match SCRUBBER.try_lock() {
        Ok(g) => g.clone(),
        Err(_) => None,
    };
    g.unwrap_or_else(|| {
        Arc::new(Scrubber::new(Scrub {
            home: home_dir_text(),
            user: std::env::var("USER").unwrap_or_default(),
            ..Scrub::default()
        }))
    })
}

/// Milliseconds from the start of the process to the first window (0 until [`mark_started`]).
pub fn startup_ms() -> Option<u64> {
    match STARTUP_MS.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

/// Notes that the window is up (first call counts).
pub fn mark_started() {
    if let Some(t) = START.get() {
        let _ = STARTUP_MS.compare_exchange(
            0,
            (t.elapsed().as_millis() as u64).max(1),
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }
}

/// Whether a formatted `tracing` line goes into the file: warnings, errors and breadcrumbs.
pub fn keeps(line: &str) -> bool {
    let mut it = line.split_whitespace();
    for _ in 0..3 {
        match it.next() {
            Some("WARN") | Some("ERROR") => return true,
            Some("INFO") => return it.next().is_some_and(|t| t.starts_with(CRUMB)),
            Some(_) => {}
            None => return false,
        }
    }
    false
}

/// Called with every formatted line (see `debug_log::Tee`).
pub fn record(line: &str) {
    if !keeps(line) {
        return;
    }
    if let Some(f) = FILE.get() {
        let clean = scrubber().apply(line);
        let _ = f.append(&clean);
    }
}

/// The panic hook: the crash file and a one-line message on stderr. The message of a panic can hold
/// anything (a path, a title in an `unwrap` on an error), so it is cleaned before it is printed or kept,
/// and the default hook, which would print it raw, is not called.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let c = CrashInfo::from_hook(info);
        let s = scrubber();
        if let Some(f) = FILE.get() {
            let _ = write_crash(f, &c, &s, chrono::Utc::now());
        }
        let _ = writeln!(
            std::io::stderr(),
            "Bookx panicked: {} at {}",
            s.apply(c.message.lines().next().unwrap_or("")),
            s.apply(c.location.as_deref().unwrap_or("?"))
        );
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-applog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn rules() -> Scrubber {
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

    #[test]
    fn only_warnings_errors_and_breadcrumbs_are_kept() {
        assert!(keeps("2026-11-22T10:00:00Z  WARN bookx::x: odd"));
        assert!(keeps("2026-11-22T10:00:00Z ERROR bookx::x: bad"));
        assert!(keeps("2026-11-22T10:00:00Z  INFO bookx_crumb: start"));
        assert!(!keeps("2026-11-22T10:00:00Z  INFO bookx::x: chatty"));
        assert!(!keeps("2026-11-22T10:00:00Z DEBUG bookx::x: noise"));
        assert!(!keeps(""));
    }

    #[test]
    fn the_log_rotates_and_never_exceeds_two_files_of_the_cap() {
        let d = tmp("rotate");
        let mut log = LogFile::new(d.clone());
        log.max = 200;
        for i in 0..100 {
            log.append(&format!("line {i:03} {}", "x".repeat(20)))
                .unwrap();
        }
        let sizes: Vec<u64> = [LOG, OLD]
            .iter()
            .map(|n| std::fs::metadata(d.join(n)).map(|m| m.len()).unwrap_or(0))
            .collect();
        assert!(sizes.iter().all(|&s| s > 0 && s <= 200), "{sizes:?}");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2);
        let tail = log.tail(1000);
        assert_eq!(
            tail.last().unwrap(),
            &format!("line 099 {}", "x".repeat(20))
        );
        assert!(tail.len() < 100, "older lines are gone");
        // the order is kept across the two files
        let nums: Vec<u32> = tail.iter().map(|l| l[5..8].parse().unwrap()).collect();
        assert!(nums.windows(2).all(|w| w[0] + 1 == w[1]), "{nums:?}");
        let _ = std::fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_is_private_and_so_are_the_files() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("perm").join("nested");
        let log = LogFile::new(d.clone());
        log.append("hello").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&d), 0o700);
        assert_eq!(mode(&log.path()), 0o600);
        let info = CrashInfo {
            message: "boom".into(),
            ..CrashInfo::default()
        };
        let p = write_crash(&log, &info, &rules(), chrono::Utc::now()).unwrap();
        assert_eq!(mode(&p), 0o600);
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    #[test]
    fn a_very_long_line_is_cut() {
        let d = tmp("long");
        let mut log = LogFile::new(d.clone());
        log.max = 1000;
        log.append(&"é".repeat(5000)).unwrap();
        assert!(std::fs::metadata(log.path()).unwrap().len() <= 1000);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_panic_writes_a_clean_crash_file_and_a_log_line() {
        let d = tmp("crash");
        let log = LogFile::new(d.clone());
        let info = CrashInfo {
            message: "cannot open The Secret Garden for alice_reads with hunter2-pass on sync.example.org: /home/alice/Books/x.epub"
                .into(),
            location: Some("/home/alice/src/foo.rs:10:5".into()),
            thread: "main".into(),
            backtrace: "   0: bookx::foo\n             at /rustc/0123456789abcdef0123456789abcdef01234567/library/core/src/x.rs:1:1\n   1: main at /home/alice/.cargo/registry/y.rs:2:2".into(),
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-22T10:11:12Z")
            .unwrap()
            .to_utc();
        let p = write_crash(&log, &info, &rules(), now).unwrap();
        assert_eq!(p.file_name().unwrap(), "crash-20261122-101112.txt");
        let t = std::fs::read_to_string(&p).unwrap();
        let l = std::fs::read_to_string(log.path()).unwrap();
        for bad in [
            "Secret Garden",
            "alice",
            "hunter2",
            "sync.example.org",
            "0123456789abcdef",
            "/home/",
        ] {
            assert!(!t.contains(bad), "{bad} leaked into the crash file:\n{t}");
            assert!(!l.contains(bad), "{bad} leaked into the log:\n{l}");
        }
        assert!(t.contains("closed unexpectedly at 2026-11-22 10:11:12 UTC"));
        assert!(t.contains("backtrace:") && t.contains("bookx::foo"));
        assert!(t.contains("/rustc/") && l.contains(" ERROR panic: "));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn crash_markers_are_found_dismissed_and_pruned() {
        let d = tmp("marker");
        let log = LogFile::new(d.clone());
        assert!(log.unseen_crash().is_none() && log.last_crash_text().is_none());
        let info = CrashInfo {
            message: "x".into(),
            ..CrashInfo::default()
        };
        let base = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        for i in 0..7 {
            write_crash(&log, &info, &rules(), base + chrono::Duration::seconds(i)).unwrap();
        }
        assert_eq!(log.crashes().len(), KEEP_CRASHES);
        let newest = log.unseen_crash().unwrap();
        assert!(newest
            .to_string_lossy()
            .ends_with("crash-20260101-000006.txt"));
        log.dismiss_crashes();
        assert!(log.unseen_crash().is_none(), "dismissed");
        assert!(
            log.last_crash_text().is_some(),
            "still available to a report"
        );
        // an old crash is not part of a new report
        let old = std::time::SystemTime::now() - CRASH_FRESH - std::time::Duration::from_secs(60);
        for p in log.crashes() {
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        assert!(log.last_crash_text().is_none(), "too old");
        // a new crash after the dismissal is unseen again
        write_crash(&log, &info, &rules(), base + chrono::Duration::seconds(99)).unwrap();
        assert!(log.unseen_crash().is_some());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_scrubber_is_always_there() {
        assert!(scrubber().apply("a b").contains("a b"));
    }

    #[test]
    fn a_crash_is_described_from_the_payload_and_location() {
        let c = CrashInfo::from_parts(&"boom", Some(("a.rs", 3, 4)), Some("main"), "bt".into());
        assert_eq!(c.message, "boom");
        assert_eq!(c.location.as_deref(), Some("a.rs:3:4"));
        assert_eq!((c.thread.as_str(), c.backtrace.as_str()), ("main", "bt"));
        let c = CrashInfo::from_parts(&String::from("owned"), None, None, String::new());
        assert_eq!(c.message, "owned");
        assert!(c.location.is_none());
        assert_eq!(c.thread, "unnamed");
        let c = CrashInfo::from_parts(&42u8, None, None, String::new());
        assert_eq!(c.message, "panic without a message");
    }

    #[test]
    fn crash_text_cuts_long_traces_and_keeps_short_rustc_paths() {
        let info = CrashInfo {
            message: "first\nsecond".into(),
            location: Some("x.rs:1:2".into()),
            thread: "t".into(),
            backtrace: format!("at /rustc/abc/lib.rs\n{}", "é".repeat(MAX_CRASH_BYTES)),
        };
        let t = crash_text(&info, "now");
        assert!(t.contains("message: first\n") && !t.contains("second"));
        assert!(t.contains("at: x.rs:1:2") && t.contains("/rustc/abc/lib.rs"));
        assert!(t.ends_with("\n(cut)\n"));
        assert!(t.len() <= MAX_CRASH_BYTES + 10);
    }

    #[test]
    fn two_crashes_in_one_second_get_two_files() {
        let d = tmp("twice");
        let log = LogFile::new(d.clone());
        let info = CrashInfo::default();
        let now = chrono::Utc::now();
        let a = write_crash(&log, &info, &rules(), now).unwrap();
        let b = write_crash(&log, &info, &rules(), now).unwrap();
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with("-1.txt"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_line_with_a_multibyte_char_at_the_cut_is_cut_on_a_boundary() {
        let d = tmp("boundary");
        let mut log = LogFile::new(d.clone());
        log.max = 10; // a quarter of it is 2 bytes: inside the first 3-byte char
        log.append("€€€€").unwrap();
        assert_eq!(std::fs::read_to_string(log.path()).unwrap(), "\n");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_process_log_records_only_what_it_keeps() {
        let d = tmp("process");
        // the only test that sets the process-wide log
        let _ = FILE.set(LogFile::new(d.clone()));
        let f = file().expect("log set");
        set_scrub(Scrub {
            user: "carol".into(),
            ..Scrub::default()
        });
        record("2026-11-22T10:00:00Z  INFO bookx::x: chatty");
        record("2026-11-22T10:00:01Z  WARN bookx::x: carol failed");
        let t = f.tail(10).join("\n");
        assert!(t.contains("WARN") && !t.contains("chatty") && !t.contains("carol"));
        assert!(state_dir().is_some() || home_dir_text().is_empty());
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn the_start_up_time_is_set_once() {
        START.get_or_init(Instant::now);
        mark_started();
        let first = startup_ms().expect("marked");
        mark_started();
        assert_eq!(startup_ms(), Some(first));
        assert!(first >= 1);
    }
}
