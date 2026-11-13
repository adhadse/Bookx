//! Library health: rolling snapshots of the catalogue, restore, and the library check
//! (roadmap PP-C-2, PP-C-16, PP-G-2).
//!
//! Snapshots are SQLite online backups in `<library>/.bookx-backups/`, so they are safe while the
//! app runs. The retention rules are a pure function ([`retention`]) so they are unit tested.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use bookx_core::book::OpenBook;
use bookx_core::library::{Book, Library};
use bookx_core::opf::Metadata;
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

use crate::catalog::LIBRARY_DB;
use crate::store::SqliteStore;

/// Folder with the snapshots, inside the library folder.
pub const BACKUP_DIR: &str = ".bookx-backups";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapshotKind {
    /// Taken by the app on start-up and exit.
    Auto,
    /// "Back up now".
    Manual,
    /// Taken right before a restore, so a restore can itself be undone.
    PreRestore,
}

impl SnapshotKind {
    fn tag(&self) -> &'static str {
        match self {
            SnapshotKind::Auto => "auto",
            SnapshotKind::Manual => "manual",
            SnapshotKind::PreRestore => "prerestore",
        }
    }

    fn from_tag(s: &str) -> Option<SnapshotKind> {
        match s {
            "auto" => Some(SnapshotKind::Auto),
            "manual" => Some(SnapshotKind::Manual),
            "prerestore" => Some(SnapshotKind::PreRestore),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SnapshotKind::Auto => "Automatic",
            SnapshotKind::Manual => "Manual",
            SnapshotKind::PreRestore => "Before a restore",
        }
    }
}

/// One snapshot file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub path: PathBuf,
    pub at: DateTime<Utc>,
    pub books: usize,
    pub kind: SnapshotKind,
    pub size: u64,
}

/// `bookx-20261112T101500Z-n123-auto.db`
pub fn snapshot_file_name(at: DateTime<Utc>, books: usize, kind: SnapshotKind) -> String {
    format!(
        "bookx-{}-n{books}-{}.db",
        at.format("%Y%m%dT%H%M%SZ"),
        kind.tag()
    )
}

pub fn parse_snapshot_name(name: &str) -> Option<(DateTime<Utc>, usize, SnapshotKind)> {
    let stem = name.strip_prefix("bookx-")?.strip_suffix(".db")?;
    let mut parts = stem.splitn(3, '-');
    let at = chrono::NaiveDateTime::parse_from_str(parts.next()?, "%Y%m%dT%H%M%SZ").ok()?;
    let books = parts.next()?.strip_prefix('n')?.parse().ok()?;
    let kind = SnapshotKind::from_tag(parts.next()?)?;
    Some((Utc.from_utc_datetime(&at), books, kind))
}

/// The snapshots of the library in `root`, newest first.
pub fn list_snapshots(root: &Path) -> Vec<Snapshot> {
    let mut out: Vec<Snapshot> = std::fs::read_dir(root.join(BACKUP_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let (at, books, kind) = parse_snapshot_name(&name)?;
            Some(Snapshot {
                path: e.path(),
                at,
                books,
                kind,
                size: e.metadata().map(|m| m.len()).unwrap_or(0),
            })
        })
        .collect();
    out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.path.cmp(&a.path)));
    out
}

/// Writes a snapshot of `store` into the library's backup folder.
pub fn take_snapshot(
    store: &SqliteStore,
    root: &Path,
    kind: SnapshotKind,
    now: DateTime<Utc>,
) -> Result<Snapshot> {
    let dir = root.join(BACKUP_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut at = now;
    let books = store.len();
    let mut path = dir.join(snapshot_file_name(at, books, kind));
    while path.exists() {
        at += Duration::seconds(1); // two snapshots in one second
        path = dir.join(snapshot_file_name(at, books, kind));
    }
    store.backup_to(&path)?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(Snapshot {
        path,
        at,
        books,
        kind,
        size,
    })
}

/// Which snapshots to delete. Manual snapshots are never deleted; of the "before a restore"
/// snapshots the newest 3 stay; of the automatic ones the newest `keep` stay plus the newest of
/// each calendar day within the last 7 days. Input order does not matter; returns indices.
pub fn retention(
    snaps: &[(DateTime<Utc>, SnapshotKind)],
    keep: usize,
    now: DateTime<Utc>,
) -> Vec<usize> {
    let mut delete = Vec::new();
    for kind in [SnapshotKind::Auto, SnapshotKind::PreRestore] {
        let mut idx: Vec<usize> = (0..snaps.len()).filter(|i| snaps[*i].1 == kind).collect();
        idx.sort_by(|a, b| snaps[*b].0.cmp(&snaps[*a].0));
        let newest = if kind == SnapshotKind::Auto { keep } else { 3 };
        let mut days: std::collections::HashSet<NaiveDate> = std::collections::HashSet::new();
        for (n, i) in idx.into_iter().enumerate() {
            let at = snaps[i].0;
            let day = NaiveDate::from_ymd_opt(at.year(), at.month(), at.day());
            let recent_day = kind == SnapshotKind::Auto
                && now - at <= Duration::days(7)
                && day.is_some_and(|d| days.insert(d));
            if n >= newest && !recent_day {
                delete.push(i);
            }
        }
    }
    delete.sort_unstable();
    delete
}

/// Deletes the snapshots [`retention`] says to. Returns how many were removed.
pub fn prune_snapshots(root: &Path, keep: usize, now: DateTime<Utc>) -> Result<usize> {
    let snaps = list_snapshots(root);
    let input: Vec<_> = snaps.iter().map(|s| (s.at, s.kind)).collect();
    let mut n = 0;
    for i in retention(&input, keep, now) {
        std::fs::remove_file(&snaps[i].path)
            .with_context(|| format!("cannot delete {}", snaps[i].path.display()))?;
        n += 1;
    }
    Ok(n)
}

/// True when the catalogue changed after the newest snapshot (or there is none yet), judged by
/// the modification time of the database and its write-ahead log.
pub fn changed_since(root: &Path, last: Option<DateTime<Utc>>) -> bool {
    let Some(last) = last else { return true };
    let mtime = |name: String| {
        std::fs::metadata(root.join(name))
            .and_then(|m| m.modified())
            .ok()
    };
    let newest = [mtime(LIBRARY_DB.into()), mtime(format!("{LIBRARY_DB}-wal"))]
        .into_iter()
        .flatten()
        .max();
    match newest {
        Some(t) => DateTime::<Utc>::from(t) > last,
        None => true,
    }
}

/// The automatic snapshot: skipped when nothing changed since the last one, then old ones are
/// pruned. Returns the new snapshot, if one was taken.
pub fn auto_snapshot(
    store: &SqliteStore,
    root: &Path,
    keep: usize,
    now: DateTime<Utc>,
) -> Result<Option<Snapshot>> {
    let last = list_snapshots(root)
        .into_iter()
        .find(|s| s.kind == SnapshotKind::Auto)
        // the snapshot file's own time, written after the copy finished
        .and_then(|s| std::fs::metadata(&s.path).and_then(|m| m.modified()).ok())
        .map(to_utc);
    if !changed_since(root, last) {
        prune_snapshots(root, keep, now)?;
        return Ok(None);
    }
    let snap = take_snapshot(store, root, SnapshotKind::Auto, now)?;
    prune_snapshots(root, keep, now)?;
    Ok(Some(snap))
}

/// Restores the catalogue from a snapshot. The snapshot is checked first, and the current
/// catalogue is snapshotted (kind [`SnapshotKind::PreRestore`]) so nothing is lost.
pub fn restore_snapshot(
    store: &SqliteStore,
    root: &Path,
    snapshot: &Path,
    now: DateTime<Utc>,
) -> Result<()> {
    let problems = SqliteStore::check_file(snapshot);
    if let Some(p) = problems.first() {
        bail!("that backup is damaged ({p}); nothing was restored");
    }
    take_snapshot(store, root, SnapshotKind::PreRestore, now)
        .context("cannot save the current catalogue before restoring")?;
    store.restore_from(snapshot)
}

// -- the library check ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    Database,
    MissingFile,
    EmptyFile,
    MissingCover,
    UnreadableFile,
    Orphan,
    Drift,
}

/// What can be done about a finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// Drop the catalogue entry of a book whose file is gone.
    RemoveEntry,
    /// Add a file that the catalogue does not know.
    ImportFile(PathBuf),
    /// The catalogue takes the metadata from the file.
    UseFile,
    /// The file takes the metadata of the catalogue.
    UseCatalogue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub kind: FindingKind,
    pub book: Option<u64>,
    pub message: String,
    pub fixes: Vec<Fix>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CheckOptions {
    /// Compare each book's metadata with the file's (opens every book).
    pub drift: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CheckReport {
    pub books_checked: usize,
    pub findings: Vec<Finding>,
}

impl CheckReport {
    pub fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count()
    }

    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The fields in which the catalogue and the file differ: `(field, catalogue, file)`.
pub fn drift_fields(cat: &Metadata, file: &Metadata) -> Vec<(&'static str, String, String)> {
    let mut out = Vec::new();
    let mut cmp = |name: &'static str, a: String, b: String| {
        if a != b {
            out.push((name, a, b));
        }
    };
    cmp("title", norm(&cat.title), norm(&file.title));
    cmp("authors", cat.authors.join(" & "), file.authors.join(" & "));
    cmp("series", norm(&cat.series), norm(&file.series));
    cmp(
        "series index",
        format!("{}", cat.series_index),
        format!("{}", file.series_index),
    );
    cmp("publisher", norm(&cat.publisher), norm(&file.publisher));
    cmp("language", norm(&cat.language), norm(&file.language));
    let tags = |m: &Metadata| {
        let mut t = m.tags.clone();
        t.sort();
        t.join(", ")
    };
    cmp("tags", tags(cat), tags(file));
    cmp("published", norm(&cat.published), norm(&file.published));
    out
}

fn is_book_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("epub"))
}

/// Book files below the library folder that no catalogue entry points to.
fn orphans(lib: &Library) -> Vec<PathBuf> {
    let known: std::collections::HashSet<PathBuf> =
        lib.books.iter().map(|b| lib.book_path(b)).collect();
    let mut out = Vec::new();
    let mut stack = vec![lib.root.clone()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let hidden = e.file_name().to_string_lossy().starts_with('.');
            if p.is_dir() {
                if !hidden {
                    stack.push(p);
                }
            } else if !hidden && is_book_file(&p) && !known.contains(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn push(
    r: &mut CheckReport,
    severity: Severity,
    kind: FindingKind,
    book: Option<u64>,
    message: String,
    fixes: Vec<Fix>,
) {
    r.findings.push(Finding {
        severity,
        kind,
        book,
        message,
        fixes,
    });
}

fn check_book(lib: &Library, b: &Book, opts: CheckOptions, r: &mut CheckReport) {
    let title = &b.meta.title;
    let path = lib.book_path(b);
    match std::fs::metadata(&path) {
        Err(_) => {
            push(
                r,
                Severity::Error,
                FindingKind::MissingFile,
                Some(b.id),
                format!("“{title}”: the file {} is missing", b.file),
                vec![Fix::RemoveEntry],
            );
            return;
        }
        Ok(m) if m.len() == 0 => {
            push(
                r,
                Severity::Error,
                FindingKind::EmptyFile,
                Some(b.id),
                format!("“{title}”: the file {} is empty", b.file),
                vec![Fix::RemoveEntry],
            );
            return;
        }
        Ok(_) => {}
    }
    if let Some(c) = &b.cover {
        if !lib.root.join(c).exists() {
            push(
                r,
                Severity::Warning,
                FindingKind::MissingCover,
                Some(b.id),
                format!("“{title}”: the cover {c} is missing"),
                vec![],
            );
        }
    }
    if opts.drift {
        match OpenBook::open(&path) {
            Err(e) => push(
                r,
                Severity::Error,
                FindingKind::UnreadableFile,
                Some(b.id),
                format!("“{title}”: the file cannot be read ({e})"),
                vec![],
            ),
            Ok(book) => {
                let diffs = drift_fields(&b.meta, book.metadata());
                if !diffs.is_empty() {
                    let list: Vec<String> = diffs
                        .iter()
                        .map(|(f, c, d)| format!("{f}: catalogue “{c}”, file “{d}”"))
                        .collect();
                    push(
                        r,
                        Severity::Warning,
                        FindingKind::Drift,
                        Some(b.id),
                        format!("“{title}” differs from its file — {}", list.join("; ")),
                        vec![Fix::UseFile, Fix::UseCatalogue],
                    );
                }
            }
        }
    }
}

/// Checks the library: the database, then every book's file and cover, files nobody knows, and
/// (with `opts.drift`) whether the file and the catalogue still agree.
pub fn check_library(
    lib: &Library,
    store: Option<&SqliteStore>,
    opts: CheckOptions,
) -> CheckReport {
    let mut r = CheckReport {
        books_checked: lib.books.len(),
        findings: Vec::new(),
    };
    if let Some(s) = store {
        for p in s.check_integrity() {
            push(
                &mut r,
                Severity::Error,
                FindingKind::Database,
                None,
                format!("The catalogue database reports: {p}. Restore a backup."),
                vec![],
            );
        }
    }
    for b in &lib.books {
        check_book(lib, b, opts, &mut r);
    }
    for p in orphans(lib) {
        let shown = p
            .strip_prefix(&lib.root)
            .unwrap_or(&p)
            .display()
            .to_string();
        push(
            &mut r,
            Severity::Info,
            FindingKind::Orphan,
            None,
            format!("{shown} is in the library folder but not in the catalogue"),
            vec![Fix::ImportFile(p)],
        );
    }
    r
}

/// Applies one of a finding's fixes. Returns a sentence saying what happened.
pub fn apply_fix(lib: &mut Library, finding: &Finding, fix: &Fix) -> Result<String> {
    match (fix, finding.book) {
        (Fix::RemoveEntry, Some(id)) => {
            lib.remove(&[id], false)?;
            Ok("Removed the catalogue entry (it is in the trash and can be restored)".into())
        }
        (Fix::ImportFile(p), _) => match lib.add_path(p, false)? {
            bookx_core::library::AddOutcome::Added(_) => Ok(format!("Added {}", p.display())),
            bookx_core::library::AddOutcome::Duplicate(id) => {
                Ok(format!("{} looks like book #{id}; not added", p.display()))
            }
        },
        (Fix::UseFile, Some(id)) => {
            let from_file = lib.open_book(id)?.metadata().clone();
            let b = lib.get_mut(id).context("no such book")?;
            b.meta = from_file;
            b.modified = Utc::now();
            lib.save()?;
            Ok("The catalogue now follows the file".into())
        }
        (Fix::UseCatalogue, Some(id)) => {
            let meta = lib.get(id).context("no such book")?.meta.clone();
            lib.update_metadata(id, meta)?;
            Ok("The file now follows the catalogue".into())
        }
        _ => bail!("that fix does not apply here"),
    }
}

/// Modification time as a `DateTime`, for tests and the UI.
pub fn to_utc(t: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap()
    }

    #[test]
    fn names_round_trip() {
        let t = Utc.with_ymd_and_hms(2026, 11, 12, 10, 15, 0).unwrap();
        let n = snapshot_file_name(t, 123, SnapshotKind::Auto);
        assert_eq!(n, "bookx-20261112T101500Z-n123-auto.db");
        assert_eq!(parse_snapshot_name(&n), Some((t, 123, SnapshotKind::Auto)));
        for k in [SnapshotKind::Manual, SnapshotKind::PreRestore] {
            assert_eq!(
                parse_snapshot_name(&snapshot_file_name(t, 0, k)).unwrap().2,
                k
            );
        }
        assert_eq!(parse_snapshot_name("bookx-library.db"), None);
        assert_eq!(parse_snapshot_name("bookx-2026-n1-auto.db"), None);
        assert_eq!(parse_snapshot_name("random.db"), None);
    }

    #[test]
    fn retention_keeps_the_newest_n_and_one_per_recent_day() {
        let now = at(12, 12);
        // five on the 12th, one each on the 11th and 10th, one a month ago
        let mut s: Vec<(DateTime<Utc>, SnapshotKind)> =
            (1..=5).map(|h| (at(12, h), SnapshotKind::Auto)).collect();
        s.push((at(11, 9), SnapshotKind::Auto));
        s.push((at(10, 9), SnapshotKind::Auto));
        s.push((
            Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap(),
            SnapshotKind::Auto,
        ));
        let del = retention(&s, 2, now);
        // kept: the 12th at 5h and 4h (newest 2), one for the 12th is already covered, the 11th and 10th
        assert_eq!(del, vec![0, 1, 2, 7]);
    }

    #[test]
    fn manual_snapshots_are_never_deleted_and_prerestore_keeps_three() {
        let now = at(12, 12);
        let mut s: Vec<(DateTime<Utc>, SnapshotKind)> =
            (1..=6).map(|d| (at(d, 1), SnapshotKind::Manual)).collect();
        s.extend((1..=5).map(|d| (at(d, 2), SnapshotKind::PreRestore)));
        let del = retention(&s, 1, now);
        assert_eq!(del, vec![6, 7], "the two oldest pre-restore snapshots");
        assert!(retention(&[], 5, now).is_empty());
        assert!(retention(&s[..6], 0, now).is_empty());
    }

    #[test]
    fn retention_ignores_input_order() {
        let now = at(12, 12);
        let s: Vec<_> = (1..=8).map(|d| (at(d, 3), SnapshotKind::Auto)).collect();
        let mut rev = s.clone();
        rev.reverse();
        let a: Vec<_> = retention(&s, 3, now).iter().map(|i| s[*i].0).collect();
        let mut b: Vec<_> = retention(&rev, 3, now).iter().map(|i| rev[*i].0).collect();
        b.sort();
        let mut a = a;
        a.sort();
        assert_eq!(a, b);
    }

    #[test]
    fn drift_lists_only_real_differences() {
        let a = Metadata {
            title: "Dune".into(),
            authors: vec!["Frank Herbert".into()],
            tags: vec!["b".into(), "a".into()],
            ..Default::default()
        };
        let mut b = a.clone();
        b.title = "  Dune ".into(); // whitespace is not a difference
        b.tags = vec!["a".into(), "b".into()]; // neither is the order of tags
        assert!(drift_fields(&a, &b).is_empty());
        b.title = "Dune Messiah".into();
        b.series = "Dune".into();
        let d = drift_fields(&a, &b);
        assert_eq!(d.len(), 2);
        assert_eq!(
            d[0],
            ("title", "Dune".to_string(), "Dune Messiah".to_string())
        );
    }
}
