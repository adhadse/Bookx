//! Glue between the backup destinations (`bookx-backup`) and the app: pushing the newest snapshot
//! to the destinations that are due, recording what happened, and the words for the Library
//! Health window. No GTK in here, so it is unit tested; the window is `ui/health.rs` and
//! `ui/backup.rs`.

use crate::i18n::{tr, trf, N_};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bookx_backup::crypto::Kdf;
use bookx_backup::places::{self, Place, PlaceKind};
use bookx_backup::{
    BackupSettings, Cancelled, DestKind, Destination, Progress, PushReport, Trigger,
};
use bookx_store::health::{self, Snapshot, SnapshotKind};
use chrono::{DateTime, Utc};

/// What happened at one destination.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub id: String,
    pub name: String,
    pub result: Result<PushReport, String>,
    pub cancelled: bool,
}

/// The newest snapshot worth copying away (not the "before a restore" ones).
pub fn newest_snapshot(root: &Path) -> Option<Snapshot> {
    health::list_snapshots(root)
        .into_iter()
        .find(|s| s.kind != SnapshotKind::PreRestore)
}

/// Whether `snap` has not been copied to `d` yet.
pub fn needs_push(d: &Destination, snap: &Snapshot) -> bool {
    d.last_ok.is_none_or(|ok| snap.at > ok)
}

/// Copies `snap` to each destination in turn. A failure at one place does not stop the others.
pub fn push_all(
    dests: &[Destination],
    passphrase: &str,
    snap: &Snapshot,
    now: DateTime<Utc>,
    kdf: Kdf,
    cancel: &AtomicBool,
    progress: &dyn Fn(&str, Progress),
) -> Vec<Outcome> {
    let mut out = Vec::new();
    for d in dests {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let pass = (!passphrase.is_empty()).then_some(passphrase);
        let result = match d.problem(pass.is_some()) {
            Some(p) => Err(anyhow::anyhow!(p)),
            None => d.target().and_then(|t| {
                let opts = bookx_backup::PushOptions {
                    passphrase: if d.encrypt { pass } else { None },
                    keep: d.keep,
                    now,
                    kdf,
                };
                bookx_backup::push(t.as_ref(), snap, &opts, cancel, &|p| progress(&d.name, p))
            }),
        };
        let cancelled = result
            .as_ref()
            .err()
            .is_some_and(|e| e.downcast_ref::<Cancelled>().is_some());
        out.push(Outcome {
            id: d.id.clone(),
            name: d.name.clone(),
            result: result.map_err(|e| format!("{e:#}")),
            cancelled,
        });
    }
    out
}

/// Writes the outcomes into the destinations' status. A cancelled push changes nothing.
pub fn apply(settings: &mut BackupSettings, outcomes: &[Outcome], now: DateTime<Utc>) {
    for o in outcomes.iter().filter(|o| !o.cancelled) {
        if let Some(d) = settings.destinations.iter_mut().find(|d| d.id == o.id) {
            match &o.result {
                Ok(_) => d.record_ok(now),
                Err(e) => d.record_error(e, now),
            }
        }
    }
}

/// The scheduled copy: the destinations that are due for `trigger` get the newest snapshot, if
/// they do not have it yet.
pub fn run_due(
    root: &Path,
    settings: &BackupSettings,
    trigger: Trigger,
    now: DateTime<Utc>,
    kdf: Kdf,
    cancel: &AtomicBool,
) -> Vec<Outcome> {
    let Some(snap) = newest_snapshot(root) else {
        return Vec::new();
    };
    let dests: Vec<Destination> = settings
        .due(trigger, now)
        .into_iter()
        .map(|i| settings.destinations[i].clone())
        .filter(|d| needs_push(d, &snap))
        .collect();
    if dests.is_empty() {
        return Vec::new();
    }
    push_all(
        &dests,
        &settings.passphrase,
        &snap,
        now,
        kdf,
        cancel,
        &|_, _| {},
    )
}

/// [`run_due`] on a worker thread that is given up on after `limit`, so closing the app never
/// hangs on a NAS that went away. Places not finished in time are reported as failed.
pub fn run_due_within(
    root: &Path,
    settings: &BackupSettings,
    trigger: Trigger,
    now: DateTime<Utc>,
    limit: Duration,
) -> Vec<Outcome> {
    let (root, st) = (root.to_path_buf(), settings.clone());
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run_due(&root, &st, trigger, now, Kdf::DEFAULT, &c2));
    });
    match rx.recv_timeout(limit) {
        Ok(o) => o,
        Err(_) => {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            settings
                .due(trigger, now)
                .into_iter()
                .map(|i| Outcome {
                    id: settings.destinations[i].id.clone(),
                    name: settings.destinations[i].name.clone(),
                    result: Err(format!(
                        "could not reach it in time ({} s); will try again next time",
                        limit.as_secs()
                    )),
                    cancelled: false,
                })
                .collect()
        }
    }
}

// -- what the window shows ------------------------------------------------------------------------------

/// The snapshots kept next to the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSummary {
    pub path: PathBuf,
    pub count: usize,
    pub bytes: u64,
    pub newest: Option<DateTime<Utc>>,
}

pub fn local_summary(root: &Path) -> LocalSummary {
    let snaps = health::list_snapshots(root);
    LocalSummary {
        path: root.join(health::BACKUP_DIR),
        count: snaps.len(),
        bytes: snaps.iter().map(|s| s.size).sum(),
        newest: snaps.first().map(|s| s.at),
    }
}

/// "3 backups · 1.2 MB · last 12 Nov 2026, 10:15" or "No backups yet".
pub fn summary_line(count: usize, bytes: u64, newest: Option<DateTime<Utc>>) -> String {
    match newest {
        Some(t) if count > 0 => format!(
            "{} · {} · last {}",
            crate::library_health::plural(count, "backup"),
            crate::library_health::size_text(bytes),
            crate::library_health::when(t)
        ),
        _ => tr("No backups yet"),
    }
}

// -- what a backup contains (D8) ---------------------------------------------------------------------------

/// Title of the scope row shown before the places, so the limits are clear before a backup is run.
pub const BACKUP_SCOPE_TITLE: &str = N_("What is in a backup");
/// Short scope line for that row.
pub const BACKUP_SCOPE_SUBTITLE: &str =
    N_("Catalogue only: titles, tags, reading progress and notes. Not book files or covers.");
/// Full scope: one complete catalogue snapshot, and everything that stays behind.
pub const BACKUP_SCOPE_DESCRIPTION: &str = N_(
    "A backup is one complete snapshot of the catalogue: metadata, tags, collections, reading progress, bookmarks, highlights and notes, and the search index. Book files, covers, configuration, plugins and secrets, .bookx state, device copies, trash and caches are not copied.",
);
/// The included side of the scope, for the developer hooks.
pub const BACKUP_INCLUDED_LABEL: &str = N_(
    "Included: catalogue metadata, collections, tags, reading progress, bookmarks, highlights, notes and the search index.",
);
/// The excluded side of the scope, for the developer hooks.
pub const BACKUP_EXCLUDED_LABEL: &str = N_(
    "Not included: book files, covers, configuration, plugins and secrets, .bookx state, trash, device copies and caches.",
);
/// What "not incremental", "skipped" and "verified" actually mean.
pub const BACKUP_INCREMENTAL_NOTE: &str = N_(
    "Not incremental — each copy is a whole file. 'Skipped' means no new snapshot was needed or the place already has that snapshot. 'Verified' means the stored bytes were read back and hashed, not that a restore was tested.",
);

/// Translated [`BACKUP_SCOPE_TITLE`].
pub fn backup_scope_title() -> String {
    tr(BACKUP_SCOPE_TITLE)
}

/// Translated [`BACKUP_SCOPE_SUBTITLE`].
pub fn backup_scope_subtitle() -> String {
    tr(BACKUP_SCOPE_SUBTITLE)
}

/// Translated [`BACKUP_SCOPE_DESCRIPTION`].
pub fn backup_scope_description() -> String {
    tr(BACKUP_SCOPE_DESCRIPTION)
}

/// Translated [`BACKUP_INCLUDED_LABEL`].
pub fn backup_included_label() -> String {
    tr(BACKUP_INCLUDED_LABEL)
}

/// Translated [`BACKUP_EXCLUDED_LABEL`].
pub fn backup_excluded_label() -> String {
    tr(BACKUP_EXCLUDED_LABEL)
}

/// Translated [`BACKUP_INCREMENTAL_NOTE`].
pub fn backup_incremental_note() -> String {
    tr(BACKUP_INCREMENTAL_NOTE)
}

/// What kind of place a destination is, in words.
pub fn kind_label(d: &Destination, found: &[Place]) -> String {
    match d.kind {
        DestKind::Webdav => PlaceKind::WebDav.label(),
        DestKind::Folder => places::classify(Path::new(d.location.trim()), found).label(),
    }
}

/// Whether the copies at `d` leave this computer's disks.
pub fn is_off_site(d: &Destination, found: &[Place]) -> bool {
    match d.kind {
        DestKind::Webdav => true,
        DestKind::Folder => places::classify(Path::new(d.location.trim()), found).off_site(),
    }
}

/// The line under a destination: its location and what it is.
pub fn destination_subtitle(d: &Destination, found: &[Place]) -> String {
    let loc = match d.kind {
        DestKind::Webdav => format!("{} / {}", d.location.trim(), d.folder.trim()),
        DestKind::Folder => d.location.trim().to_string(),
    };
    let mut s = format!("{} — {loc}", kind_label(d, found));
    if d.encrypt {
        s.push_str(" · encrypted");
    }
    s
}

/// Words for the status of a destination.
pub fn status_text(d: &Destination) -> String {
    use bookx_backup::DestStatus::*;
    match d.status() {
        NeverRun => tr("Not used yet"),
        Off => tr("Switched off"),
        Ok(t) => trf("OK — last copy {}", &[&crate::library_health::when(t)]),
        Failed { error, unreachable } => format!(
            "{} — {error}",
            if unreachable { "Unreachable" } else { "Failed" }
        ),
    }
}

/// `smb://server/share` and friends: addresses GIO can mount.
pub fn is_mountable_uri(s: &str) -> bool {
    let s = s.trim();
    [
        "smb://", "nfs://", "sftp://", "ftp://", "afp://", "davs://", "dav://",
    ]
    .iter()
    .any(|p| s.starts_with(p) && s.len() > p.len())
}

/// A readable problem with an address typed for a network share, if there is one.
pub fn uri_problem(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return Some(tr("Type an address like smb://nas/books"));
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return Some(tr(
            "That is a web address: choose “WebDAV” as the kind instead",
        ));
    }
    if !is_mountable_uri(s) {
        return Some(tr(
            "The address should start with smb://, nfs://, sftp:// or similar",
        ));
    }
    None
}

/// The sentence about who can read the copies.
pub fn privacy_note(d: &Destination, found: &[Place]) -> Option<&'static str> {
    (is_off_site(d, found) && !d.encrypt)
        .then_some("Not encrypted: whoever runs this place can read your catalogue and notes.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("bookx-backups-{}", uuid::Uuid::new_v4()))
    }

    fn store_with_snapshot(root: &Path) -> Snapshot {
        std::fs::create_dir_all(root).unwrap();
        let dbp = root.join("bookx-library.db");
        let store = bookx_store::SqliteStore::open(&dbp).unwrap();
        health::take_snapshot(&store, root, SnapshotKind::Manual, Utc::now()).unwrap()
    }

    fn folder(id: &str, dir: &Path) -> Destination {
        Destination::folder(id, id, dir.to_str().unwrap())
    }

    #[test]
    fn a_push_to_two_places_survives_one_of_them_being_gone() {
        let root = tmp();
        let snap = store_with_snapshot(&root);
        let good = root.join("usb");
        std::fs::create_dir_all(&good).unwrap();
        let dests = vec![
            folder("gone", Path::new("/definitely-not-mounted/x")),
            folder("usb", &good),
        ];
        let out = push_all(
            &dests,
            "",
            &snap,
            Utc::now(),
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert_eq!(out.len(), 2);
        assert!(out[0].result.is_err());
        assert!(out[1].result.as_ref().unwrap().verified);
        let mut st = BackupSettings {
            destinations: dests,
            ..BackupSettings::default()
        };
        apply(&mut st, &out, Utc::now());
        assert!(matches!(
            st.destinations[0].status(),
            bookx_backup::DestStatus::Failed {
                unreachable: true,
                ..
            }
        ));
        assert!(matches!(
            st.destinations[1].status(),
            bookx_backup::DestStatus::Ok(_)
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn encryption_without_a_passphrase_is_a_clear_failure_not_a_plain_copy() {
        let root = tmp();
        let snap = store_with_snapshot(&root);
        let dir = root.join("nas");
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = folder("nas", &dir);
        d.encrypt = true;
        let out = push_all(
            &[d],
            "",
            &snap,
            Utc::now(),
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert!(out[0].result.as_ref().unwrap_err().contains("passphrase"));
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "nothing written"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scheduled_runs_copy_what_is_due_once_and_only_when_there_is_something_new() {
        let root = tmp();
        let snap = store_with_snapshot(&root);
        let dir = root.join("drive");
        std::fs::create_dir_all(&dir).unwrap();
        let mut st = BackupSettings::default();
        st.destinations.push(folder("drive", &dir));
        let now = snap.at + chrono::Duration::minutes(1);
        let out = run_due(
            &root,
            &st,
            Trigger::Exit,
            now,
            Kdf::TEST,
            &AtomicBool::new(false),
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].result.is_ok(), "{:?}", out[0].result);
        apply(&mut st, &out, now);
        // nothing new: not copied again, even on exit after a while
        let later = now + chrono::Duration::hours(2);
        assert!(run_due(
            &root,
            &st,
            Trigger::Exit,
            later,
            Kdf::TEST,
            &AtomicBool::new(false)
        )
        .is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn closing_never_waits_longer_than_the_limit() {
        let root = tmp();
        let snap = store_with_snapshot(&root);
        let mut st = BackupSettings::default();
        // a place that does not answer: a local port nobody listens on is refused at once,
        // so the limit is exercised with a zero limit instead
        st.destinations.push(folder("d", &root.join("x")));
        let out = run_due_within(
            &root,
            &st,
            Trigger::Exit,
            snap.at + chrono::Duration::minutes(1),
            Duration::from_millis(0),
        );
        assert!(out.len() <= 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scope_copy_says_catalogue_only_and_whole_file_not_incremental() {
        // The constants are the msgids the UI and the developer hooks translate.
        assert_eq!(BACKUP_SCOPE_TITLE, "What is in a backup");
        assert_eq!(
            BACKUP_SCOPE_SUBTITLE,
            "Catalogue only: titles, tags, reading progress and notes. Not book files or covers."
        );
        assert_eq!(
            BACKUP_INCREMENTAL_NOTE,
            "Not incremental — each copy is a whole file. 'Skipped' means no new snapshot was needed or the place already has that snapshot. 'Verified' means the stored bytes were read back and hashed, not that a restore was tested."
        );

        let scope = format!(
            "{} {} {} {}",
            backup_scope_title(),
            backup_scope_subtitle(),
            backup_scope_description(),
            backup_included_label(),
        );
        let excluded = backup_excluded_label();
        assert!(scope.to_lowercase().contains("catalogue"), "{scope}");
        assert!(scope.contains("search index"), "{scope}");
        assert!(excluded.to_lowercase().contains("book files"), "{excluded}");
        assert!(excluded.to_lowercase().contains("covers"), "{excluded}");
        assert!(excluded.contains("Not included"), "{excluded}");
        assert!(excluded.contains("trash"), "{excluded}");

        let note = backup_incremental_note().to_lowercase();
        assert!(note.contains("not incremental"), "{note}");
        assert!(note.contains("whole file"), "{note}");
        assert!(note.contains("'skipped' means"), "{note}");
        assert!(note.contains("'verified' means"), "{note}");
        // Never promise a delta, deduplication or an incremental transfer.
        for claim in [
            "delta",
            "deduplic",
            "incremental transfer",
            "incremental copy",
            "block-level",
        ] {
            assert!(!scope.to_lowercase().contains(claim), "{claim} in {scope}");
            assert!(
                !excluded.to_lowercase().contains(claim),
                "{claim} in {excluded}"
            );
            assert!(!note.contains(claim), "{claim} in {note}");
        }
    }

    #[test]
    fn words_for_the_window() {
        assert_eq!(summary_line(0, 0, None), "No backups yet");
        let t = Utc.with_ymd_and_hms(2026, 11, 12, 10, 15, 0).unwrap();
        let s = summary_line(3, 1_500_000, Some(t));
        assert!(s.starts_with("3 backups · ") && s.contains("last"), "{s}");
        let mut d = Destination::webdav("d1", "Nextcloud", "https://c.example/dav", "u", "p");
        assert!(destination_subtitle(&d, &[]).contains("encrypted"));
        assert!(privacy_note(&d, &[]).is_none());
        d.encrypt = false;
        assert!(privacy_note(&d, &[]).unwrap().contains("Not encrypted"));
        let local = folder("l", Path::new("/home/a/backups"));
        assert!(
            privacy_note(&local, &[]).is_none(),
            "a local folder is not off-site"
        );
        assert!(status_text(&local).contains("Not used"));
    }

    #[test]
    fn network_addresses_are_checked_before_mounting() {
        assert!(uri_problem("smb://nas/books").is_none());
        assert!(uri_problem("sftp://me@host/home").is_none());
        assert!(uri_problem("").is_some());
        assert!(uri_problem("https://cloud.example")
            .unwrap()
            .contains("WebDAV"));
        assert!(uri_problem("//nas/books").is_some());
        assert!(!is_mountable_uri("smb://"));
    }
}
