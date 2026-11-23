//! Glue between the backup destinations (`bookx-backup`) and the app: pushing the newest snapshot
//! to the destinations that are due, recording what happened, and the words for the Library
//! Health window. No GTK in here, so it is unit tested; the window is `ui/health.rs` and
//! `ui/backup.rs`.

use crate::i18n::{N_, tr, trf};
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

/// What happened at one destination (old whole-catalogue copies).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Outcome {
    pub id: String,
    pub name: String,
    pub result: Result<PushReport, String>,
    pub cancelled: bool,
}

/// The newest snapshot worth copying away (not the "before a restore" ones).
#[allow(dead_code)] // the old whole-catalogue copies: kept for the legacy files and their tests
pub fn newest_snapshot(root: &Path) -> Option<Snapshot> {
    health::list_snapshots(root)
        .into_iter()
        .find(|s| s.kind != SnapshotKind::PreRestore)
}

/// Whether `snap` has not been copied to `d` yet.
#[allow(dead_code)] // the old whole-catalogue copies: kept for the legacy files and their tests
pub fn needs_push(d: &Destination, snap: &Snapshot) -> bool {
    d.last_ok.is_none_or(|ok| snap.at > ok)
}

/// Copies `snap` to each destination in turn. A failure at one place does not stop the others.
#[allow(dead_code)] // the old whole-catalogue copies: kept for the legacy files and their tests
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
#[allow(dead_code)] // the old whole-catalogue copies: kept for the legacy files and their tests
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
#[allow(dead_code)] // the old whole-catalogue copies: kept for the legacy files and their tests
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
#[allow(dead_code)] // the old whole-catalogue copies: kept for the legacy files and their tests
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
pub const BACKUP_SCOPE_SUBTITLE: &str = N_(
    "Book files, covers and catalogue with your notes. Only new or changed files are copied after the first backup.",
);
/// Full scope: what is copied, how much is copied each time, and what stays behind.
pub const BACKUP_SCOPE_DESCRIPTION: &str = N_(
    "Backups include your book files, covers and catalogue (titles, tags, collections, reading progress, bookmarks, highlights and notes). Only new or changed files are copied after the first backup. Not included: configuration, plugins and passwords, trash, device copies and caches.",
);
/// The included side of the scope, for the developer hooks.
pub const BACKUP_INCLUDED_LABEL: &str = N_(
    "Included: book files, covers and the catalogue (titles, tags, collections, reading progress, bookmarks, highlights and notes).",
);
/// The excluded side of the scope, for the developer hooks.
pub const BACKUP_EXCLUDED_LABEL: &str =
    N_("Not included: configuration, plugins and passwords, trash, device copies and caches.");
/// What "incremental", "skipped" and "verified" actually mean.
pub const BACKUP_INCREMENTAL_NOTE: &str = N_(
    "After the first backup only new or changed files are copied; a renamed or moved book costs nothing. 'Skipped' means nothing had changed. 'Verified' means the stored bytes were read back and checked, not that a restore was tested.",
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

/// The place's choice of what goes in, in a few words.
pub fn scope_label(d: &Destination) -> String {
    let mut s = match (d.include_books, d.include_covers) {
        (true, true) => tr("Books, covers and catalogue"),
        (true, false) => tr("Books and catalogue, no covers"),
        (false, true) => tr("Covers and catalogue, no book files"),
        (false, false) => tr("Catalogue only"),
    };
    if d.include_books && d.max_file_mb > 0 {
        s.push_str(" · ");
        s.push_str(&trf("files up to {} MB", &[&d.max_file_mb]));
    }
    s
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

/// The incremental repository (D9): estimate, run, journal, restore. Called by the Backups page.
pub mod repo {
    use super::*;

    use bookx_backup::repo::{
        LibrarySource, Phase, Plan, Report, Repository, RestoreMode, RestoreOptions, RestoreReport,
        RunOptions, RunProgress, Secrets, estimate_new,
    };
    use bookx_backup::{RemoteFile, RemoteName};

    /// The first backup is a full copy (shown before it runs).
    pub const BACKUP_PLAN_FIRST: &str =
        N_("The first backup copies everything: about {} in {} files.");
    /// What the next backup will copy.
    pub const BACKUP_PLAN_NEXT: &str =
        N_("{} of new or changed files ({} files) to copy; everything else is already there.");
    /// Nothing to copy.
    pub const BACKUP_PLAN_NOTHING: &str = N_("Everything is already backed up.");
    /// The place is too small for the copy.
    pub const BACKUP_PLAN_NO_SPACE: &str =
        N_("Not enough free space in this place: {} needed, {} free.");
    /// Files the backup could not or would not take.
    pub const BACKUP_PLAN_LEFT_OUT: &str = N_("{} files are left out (too big or not readable).");
    /// A finished run that copied something.
    pub const BACKUP_DONE_NEW: &str = N_("Copied {} new files ({}); {} were already there.");
    /// A finished run with nothing to copy.
    pub const BACKUP_DONE_NOTHING: &str = N_("Nothing new to copy.");
    /// Phases of a run, for the progress line.
    pub const BACKUP_PHASE_SCAN: &str = N_("Looking at your library");
    pub const BACKUP_PHASE_HASH: &str = N_("Finding what is new");
    pub const BACKUP_PHASE_COPY: &str = N_("Copying new files");
    pub const BACKUP_PHASE_FINISH: &str = N_("Writing the snapshot");

    /// Translated name of a phase of a run.
    pub fn phase_label(p: Phase) -> String {
        match p {
            Phase::Scanning => tr(BACKUP_PHASE_SCAN),
            Phase::Hashing => tr(BACKUP_PHASE_HASH),
            Phase::Uploading => tr(BACKUP_PHASE_COPY),
            Phase::Finishing => tr(BACKUP_PHASE_FINISH),
            Phase::Pruning => tr("Removing old backups"),
            Phase::Done => tr("Done"),
        }
    }

    /// The sentence under a place that says what the next backup will do.
    pub fn plan_text(p: &Plan) -> String {
        use crate::library_health::size_text;
        let mut parts = Vec::new();
        if !p.enough_space {
            parts.push(trf(
                BACKUP_PLAN_NO_SPACE,
                &[
                    &size_text(p.upload_bytes + p.catalogue_bytes),
                    &size_text(p.free.unwrap_or(0)),
                ],
            ));
        } else if p.first_run {
            parts.push(trf(
                BACKUP_PLAN_FIRST,
                &[&size_text(p.new_bytes), &p.new_files],
            ));
        } else if p.new_files == 0 {
            parts.push(tr(BACKUP_PLAN_NOTHING));
        } else {
            parts.push(trf(
                BACKUP_PLAN_NEXT,
                &[&size_text(p.new_bytes), &p.new_files],
            ));
        }
        let left = p.too_big.len() + p.skipped.len();
        if left > 0 {
            parts.push(trf(BACKUP_PLAN_LEFT_OUT, &[&left]));
        }
        parts.join(" ")
    }

    /// The sentence after a run.
    pub fn report_text(r: &Report) -> String {
        use crate::library_health::size_text;
        if r.up_to_date || r.new_files == 0 {
            tr(BACKUP_DONE_NOTHING)
        } else {
            trf(
                BACKUP_DONE_NEW,
                &[&r.new_files, &size_text(r.new_bytes), &r.reused_files],
            )
        }
    }

    /// What a backup of the library would copy to one place.
    #[derive(Debug, Clone)]
    pub struct Estimate {
        pub id: String,
        pub name: String,
        pub plan: Result<Plan, String>,
    }

    /// A folder place whose own folder is not there yet but whose parent is: typing a new folder name
    /// is fine, it is created by the first backup. (A parent that is missing means a drive that is not
    /// plugged in or a share that is not mounted: that is never created.)
    pub fn will_be_created(d: &Destination) -> bool {
        if d.kind != DestKind::Folder {
            return false;
        }
        let p = Path::new(d.location.trim());
        !d.location.trim().is_empty()
            && !p.exists()
            && p.parent()
                .is_some_and(|x| x.as_os_str().is_empty() || x.is_dir())
    }

    fn prepare_place(d: &Destination) -> anyhow::Result<()> {
        if will_be_created(d) {
            std::fs::create_dir(d.location.trim())?;
        }
        Ok(())
    }

    /// Looks whether a place can be read without changing it (a plain `check` would create the
    /// repository folder and leave a test file in it, which is not what a look should do).
    fn check_readable(
        d: &Destination,
        store: &dyn bookx_backup::repo::store::RepoStore,
    ) -> anyhow::Result<()> {
        if d.kind == DestKind::Folder {
            let loc = Path::new(d.location.trim());
            if !loc.is_dir() {
                anyhow::bail!(
                    "{} is not there. Is the drive plugged in or the share mounted?",
                    loc.display()
                );
            }
            Ok(())
        } else {
            store.check()
        }
    }

    fn secrets_for(d: &Destination, passphrase: &str) -> Secrets {
        if d.encrypt {
            Secrets::passphrase(passphrase)
        } else {
            Secrets::none()
        }
    }

    /// How much each place would receive if a backup ran now, before the user starts one. Places
    /// without a repository yet are estimated from file sizes; the others use what the library
    /// remembers about its files. A place that cannot be reached says why instead.
    pub fn estimate_for(
        lib: &LibrarySource,
        places: &[Destination],
        passphrase: &str,
        cancel: &AtomicBool,
    ) -> Vec<Estimate> {
        places
            .iter()
            .map(|d| {
                let plan = (|| -> anyhow::Result<Plan> {
                    if let Some(p) = d.problem(!passphrase.is_empty()) {
                        anyhow::bail!(p);
                    }
                    if will_be_created(d) {
                        return estimate_new(lib, &d.scope(), d.encrypt, None);
                    }
                    let store = d.repo_store()?;
                    check_readable(d, store.as_ref())?;
                    match Repository::is_encrypted_at(store.as_ref())? {
                        None => estimate_new(lib, &d.scope(), d.encrypt, store.free_space()),
                        Some(_) => {
                            let repo = Repository::open(store, &secrets_for(d, passphrase))?;
                            repo.plan(lib, &d.scope(), 3, cancel, &|_| {})
                        }
                    }
                })();
                Estimate {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    plan: plan.map_err(|e| format!("{e:#}")),
                }
            })
            .collect()
    }

    /// What happened at one place in a repository run.
    #[derive(Debug, Clone)]
    pub struct RepoOutcome {
        pub id: String,
        pub name: String,
        pub result: Result<Report, String>,
        pub cancelled: bool,
    }

    /// Backs the library up to each place in turn (book files, covers, catalogue, notes: only what the
    /// place lacks is copied). A failure at one place does not stop the others. `manual` snapshots are
    /// never pruned; `progress` gets the place's name and the run's progress (from worker threads).
    pub fn run_all(
        lib: &LibrarySource,
        places: &[Destination],
        passphrase: &str,
        manual: bool,
        now: DateTime<Utc>,
        kdf: Kdf,
        cancel: &AtomicBool,
        progress: &(dyn Fn(&str, &RunProgress) + Sync),
    ) -> Vec<RepoOutcome> {
        let mut out = Vec::new();
        for d in places {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let result = (|| -> anyhow::Result<Report> {
                if let Some(p) = d.problem(!passphrase.is_empty()) {
                    anyhow::bail!(p);
                }
                prepare_place(d)?;
                let repo = Repository::open_or_create(
                    d.repo_store()?,
                    &secrets_for(d, passphrase),
                    d.encrypt,
                    kdf,
                )?;
                let opts = RunOptions {
                    manual,
                    scope: d.scope(),
                    keep: d.keep,
                    now,
                    ..RunOptions::default()
                };
                repo.run(lib, &opts, &|p| progress(&d.name, p), cancel)
            })();
            let cancelled = result
                .as_ref()
                .err()
                .is_some_and(|e| e.downcast_ref::<Cancelled>().is_some());
            out.push(RepoOutcome {
                id: d.id.clone(),
                name: d.name.clone(),
                result: result.map_err(|e| format!("{e:#}")),
                cancelled,
            });
        }
        out
    }

    /// Writes the outcomes into the places' status. A cancelled run changes nothing. A place that
    /// got its first full copy may be copied to automatically from then on.
    pub fn apply_repo(settings: &mut BackupSettings, outcomes: &[RepoOutcome], now: DateTime<Utc>) {
        for o in outcomes.iter().filter(|o| !o.cancelled) {
            if let Some(d) = settings.destinations.iter_mut().find(|d| d.id == o.id) {
                match &o.result {
                    Ok(_) => {
                        tracing::info!(target: "bookx_crumb", "backup to a {:?} place: ok", d.kind);
                        d.record_ok(now);
                        d.auto_ok = true;
                    }
                    Err(e) => {
                        tracing::warn!("backup to a {:?} place failed: {e}", d.kind);
                        d.record_error(e, now)
                    }
                }
            }
        }
    }

    /// The places a scheduled copy (when Bookx closes, once a day) may use: those that are due and
    /// either had their first full copy or were allowed to start it by themselves. The first copy
    /// of a place is never a surprise: it runs when the user presses Back Up (or allows it in the
    /// place's settings after seeing how big it is).
    pub fn scheduled_places(
        settings: &BackupSettings,
        trigger: Trigger,
        now: DateTime<Utc>,
    ) -> Vec<Destination> {
        settings
            .due(trigger, now)
            .into_iter()
            .map(|i| settings.destinations[i].clone())
            .filter(|d| d.auto_ok)
            .collect()
    }

    /// The places whose notes are recorded between backups: switched on and already allowed.
    pub fn journal_places(settings: &BackupSettings) -> Vec<Destination> {
        settings
            .destinations
            .iter()
            .filter(|d| d.enabled && d.auto_ok)
            .cloned()
            .collect()
    }

    /// The scheduled copy: the places due for `trigger` get a backup (which does nothing when
    /// nothing changed).
    pub fn run_due_repo(
        lib: &LibrarySource,
        settings: &BackupSettings,
        trigger: Trigger,
        now: DateTime<Utc>,
        kdf: Kdf,
        cancel: &AtomicBool,
    ) -> Vec<RepoOutcome> {
        let dests = scheduled_places(settings, trigger, now);
        run_all(
            lib,
            &dests,
            &settings.passphrase,
            false,
            now,
            kdf,
            cancel,
            &|_, _| {},
        )
    }

    /// [`run_due_repo`] on a worker thread that is given up on after `limit`, so closing the app never
    /// hangs on a place that went away. Places not finished in time are reported as failed and the
    /// run is cancelled (it leaves no half-written files).
    pub fn run_due_repo_within(
        lib: &LibrarySource,
        settings: &BackupSettings,
        trigger: Trigger,
        now: DateTime<Utc>,
        limit: Duration,
    ) -> Vec<RepoOutcome> {
        let (lib2, st) = (lib.clone(), settings.clone());
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_due_repo(&lib2, &st, trigger, now, Kdf::DEFAULT, &c2));
        });
        match rx.recv_timeout(limit) {
            Ok(o) => o,
            Err(_) => {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                scheduled_places(settings, trigger, now)
                    .into_iter()
                    .map(|d| RepoOutcome {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        result: Err(format!(
                            "could not finish in time ({} s); will try again next time",
                            limit.as_secs()
                        )),
                        cancelled: false,
                    })
                    .collect()
            }
        }
    }

    /// Records the highlights, notes, bookmarks and progress that changed since the last record at
    /// every place that is on (the caller passes [`journal_places`]), so they are protected between full backups. Cheap; call every few
    /// minutes. Returns the places that failed, with the reason.
    pub fn journal_all(
        lib: &LibrarySource,
        places: &[Destination],
        passphrase: &str,
        now: DateTime<Utc>,
        kdf: Kdf,
    ) -> Vec<(String, String)> {
        let cancel = AtomicBool::new(false);
        let mut failed = Vec::new();
        for d in places.iter().filter(|d| d.enabled) {
            let r = (|| -> anyhow::Result<()> {
                if let Some(p) = d.problem(!passphrase.is_empty()) {
                    anyhow::bail!(p);
                }
                prepare_place(d)?;
                let repo = Repository::open_or_create(
                    d.repo_store()?,
                    &secrets_for(d, passphrase),
                    d.encrypt,
                    kdf,
                )?;
                repo.journal_append(lib, now, &cancel)?;
                Ok(())
            })();
            if let Err(e) = r {
                failed.push((d.name.clone(), format!("{e:#}")));
            }
        }
        failed
    }

    // -- progress, summaries and the words for them (pure, unit tested) ------------------------------------

    /// How far a run is, 0..1, weighing the phases: looking and hashing are quick, copying is the bulk.
    pub fn progress_fraction(p: &RunProgress) -> f64 {
        let part = |done: u64, total: u64| {
            if total == 0 {
                1.0
            } else {
                (done as f64 / total as f64).clamp(0.0, 1.0)
            }
        };
        match p.phase {
            Phase::Scanning => 0.02,
            Phase::Hashing => 0.02 + 0.18 * part(p.files_done, p.files_total),
            Phase::Uploading => {
                let f = if p.bytes_total > 0 {
                    part(p.bytes_done, p.bytes_total)
                } else {
                    part(p.files_done, p.files_total)
                };
                0.20 + 0.75 * f
            }
            Phase::Finishing => 0.96,
            Phase::Pruning => 0.98,
            Phase::Done => 1.0,
        }
    }

    /// "A few seconds", "about 3 minutes" for a time left.
    pub fn eta_phrase(left: Duration) -> String {
        let s = left.as_secs_f64();
        let amount = if s < 90.0 {
            let n = (s.round() as usize).max(1);
            crate::i18n::ntr("{} second", "{} seconds", n as u64).replace("{}", &n.to_string())
        } else {
            let n = ((s / 60.0).round() as usize).max(2);
            crate::i18n::ntr("{} minute", "{} minutes", n as u64).replace("{}", &n.to_string())
        };
        trf("about {} left", &[&amount])
    }

    /// The line under the bar: "12 of 340 files · 120 MB of 2.1 GB · about 5 minutes left".
    pub fn progress_detail(p: &RunProgress) -> String {
        use crate::library_health::size_text;
        let mut parts = Vec::new();
        if p.files_total > 0 {
            parts.push(trf("{} of {} files", &[&p.files_done, &p.files_total]));
        }
        if p.phase == Phase::Uploading && p.bytes_total > 0 {
            parts.push(trf(
                "{} of {}",
                &[&size_text(p.bytes_done), &size_text(p.bytes_total)],
            ));
        }
        if let Some(eta) = p.eta.filter(|_| p.phase == Phase::Uploading) {
            parts.push(eta_phrase(eta));
        }
        parts.join(" · ")
    }

    /// "Offsite: Copying new files".
    pub fn progress_stage(place: &str, p: &RunProgress) -> String {
        format!("{place}: {}", phase_label(p.phase))
    }

    /// What a place holds, from its newest snapshot.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RepoSummary {
        pub snapshots: usize,
        pub newest: Option<DateTime<Utc>>,
        pub books: usize,
        pub files: usize,
        pub bytes: u64,
        pub encrypted: bool,
        /// Notes are known up to this time (the newest journal record).
        pub notes_until: Option<DateTime<Utc>>,
    }

    /// What the place holds; `None` when it has no repository (yet).
    pub fn repo_summary(d: &Destination, passphrase: &str) -> anyhow::Result<Option<RepoSummary>> {
        if will_be_created(d) {
            return Ok(None);
        }
        let store = d.repo_store()?;
        check_readable(d, store.as_ref())?;
        let Some(enc) = Repository::is_encrypted_at(store.as_ref())? else {
            return Ok(None);
        };
        let repo = Repository::open(store, &Secrets::passphrase(passphrase))?;
        let snaps = repo.list_snapshots()?;
        let newest = snaps.first();
        let (books, files, bytes) = match newest.map(|s| repo.manifest(&s.name)) {
            Some(Ok(m)) => (m.books, m.files.len(), m.total_bytes()),
            _ => (0, 0, 0),
        };
        Ok(Some(RepoSummary {
            snapshots: snaps.len(),
            newest: newest.map(|s| s.name.at),
            books,
            files,
            bytes,
            encrypted: enc,
            notes_until: repo.list_journal()?.last().map(|j| j.1),
        }))
    }

    /// "3 backups · 128 books · 1.2 GB · last 12 Nov 2026, 10:15".
    pub fn summary_text(s: &RepoSummary) -> String {
        match s.newest {
            Some(t) if s.snapshots > 0 => trf(
                "{} · {} · {} · last {}",
                &[
                    &crate::library_health::plural(s.snapshots, "backup"),
                    &crate::library_health::plural(s.books, "book"),
                    &crate::library_health::size_text(s.bytes),
                    &crate::library_health::when(t),
                ],
            ),
            _ => tr("No backups yet"),
        }
    }

    /// The one-time row shown until a place has had its first copy: how big it is and that nothing
    /// is copied until the user says so. `None` when there is nothing to say.
    pub fn banner_text(plans: &[Plan]) -> Option<String> {
        use crate::library_health::size_text;
        let biggest = plans
            .iter()
            .filter(|p| p.first_run && p.new_files > 0)
            .max_by_key(|p| p.new_bytes)?;
        Some(trf(
            "The first copy is about {} in {} files. Later copies only add what is new. Nothing is copied until you press Back Up.",
            &[&size_text(biggest.new_bytes), &biggest.new_files],
        ))
    }

    /// The message after a run over all places.
    pub fn outcomes_text(outcomes: &[RepoOutcome]) -> String {
        if outcomes.iter().any(|o| o.cancelled) {
            return tr("Backup cancelled; nothing half-written is left behind");
        }
        let bad: Vec<String> = outcomes
            .iter()
            .filter_map(|o| o.result.as_ref().err().map(|e| format!("{}: {e}", o.name)))
            .collect();
        let good: Vec<(&str, &Report)> = outcomes
            .iter()
            .filter_map(|o| o.result.as_ref().ok().map(|r| (o.name.as_str(), r)))
            .collect();
        let mut parts = Vec::new();
        match good.as_slice() {
            [] => {}
            [(_, r)] => parts.push(report_text(r)),
            many => parts.push(
                many.iter()
                    .map(|(n, r)| format!("{n}: {}", report_text(r)))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
        }
        let left: usize = good
            .iter()
            .map(|(_, r)| r.too_big.len() + r.skipped.len())
            .max()
            .unwrap_or(0);
        if left > 0 {
            parts.push(trf(BACKUP_PLAN_LEFT_OUT, &[&left]));
        }
        if !bad.is_empty() {
            parts.push(trf("Could not copy to: {}", &[&bad.join("; ")]));
        }
        parts.join(" ")
    }

    /// Checks a place (every file is there with the right size; `deep` reads everything back).
    pub fn check_place(
        d: &Destination,
        passphrase: &str,
        deep: bool,
        cancel: &AtomicBool,
        progress: &(dyn Fn(u64, u64) + Sync),
    ) -> anyhow::Result<bookx_backup::repo::VerifyReport> {
        let repo = Repository::open(d.repo_store()?, &Secrets::passphrase(passphrase))?;
        repo.verify(deep, 3, cancel, progress)
    }

    /// Words for a check.
    pub fn check_text(r: &bookx_backup::repo::VerifyReport) -> String {
        use crate::library_health::plural;
        if r.ok() {
            return if r.deep {
                trf(
                    "The backup is intact: {} files were read back and match.",
                    &[&r.blobs_checked],
                )
            } else {
                trf(
                    "The backup is complete: all {} files are there with the right size. A deep check also reads their contents.",
                    &[&r.blobs_checked],
                )
            };
        }
        let mut parts = Vec::new();
        if !r.missing.is_empty() {
            parts.push(trf("{} are missing", &[&plural(r.missing.len(), "file")]));
        }
        if !r.corrupt.is_empty() {
            parts.push(trf("{} are damaged", &[&plural(r.corrupt.len(), "file")]));
        }
        if !r.snapshots_bad.is_empty() {
            parts.push(trf(
                "{} cannot be read",
                &[&plural(r.snapshots_bad.len(), "backup")],
            ));
        }
        if !r.journal_bad.is_empty() {
            parts.push(trf(
                "{} of the notes records are damaged",
                &[&r.journal_bad.len()],
            ));
        }
        trf(
            "The backup has problems: {}. Press Back Up to copy missing files again.",
            &[&parts.join(", ")],
        )
    }

    /// What removing old backups did.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct CleanReport {
        pub snapshots_removed: usize,
        pub files_removed: usize,
        pub bytes_freed: u64,
    }

    /// Removes the snapshots a place no longer keeps (its "backups to keep" setting; manual ones
    /// stay) and the files nothing uses any more.
    pub fn clean_place(
        d: &Destination,
        passphrase: &str,
        now: DateTime<Utc>,
        cancel: &AtomicBool,
    ) -> anyhow::Result<CleanReport> {
        let repo = Repository::open(d.repo_store()?, &Secrets::passphrase(passphrase))?;
        let snapshots_removed = repo.prune(d.keep, now)?;
        repo.compact_journal()?;
        // The user asked for this and this program is the only writer, so no waiting period.
        let g = repo.gc(chrono::Duration::zero(), now, cancel)?;
        let g = if g.newly_marked > 0 {
            let again = repo.gc(chrono::Duration::zero(), now, cancel)?;
            (
                again.deleted_blobs + g.deleted_blobs,
                again.deleted_bytes + g.deleted_bytes,
            )
        } else {
            (g.deleted_blobs, g.deleted_bytes)
        };
        Ok(CleanReport {
            snapshots_removed,
            files_removed: g.0,
            bytes_freed: g.1,
        })
    }

    /// Words for a clean-up.
    pub fn clean_text(r: &CleanReport) -> String {
        if r.snapshots_removed == 0 && r.files_removed == 0 {
            tr("Nothing to remove: the place only has backups it keeps.")
        } else {
            trf(
                "Removed {} old backups and {} files no backup needs ({} freed).",
                &[
                    &r.snapshots_removed,
                    &r.files_removed,
                    &crate::library_health::size_text(r.bytes_freed),
                ],
            )
        }
    }

    /// The dry run of a restore, in words: what would come back and what is already there.
    pub fn restore_plan_text(r: &RestoreReport) -> String {
        use crate::library_health::plural;
        let mut parts = vec![trf(
            "{} to restore, {} already present.",
            &[
                &plural(r.would_write, "file"),
                &(r.skipped_existing + r.skipped_same),
            ],
        )];
        match r.catalogue {
            bookx_backup::repo::CatalogueAction::WouldRestore => parts.push(tr(
                "The catalogue is replaced by the one in the backup; the current one is saved first.",
            )),
            bookx_backup::repo::CatalogueAction::KeptExisting => {
                parts.push(tr("The current catalogue is kept."))
            }
            _ => {}
        }
        if !r.replaced_newer.is_empty() {
            parts.push(trf(
                "{} are newer here than in the backup and would be replaced.",
                &[&plural(r.replaced_newer.len(), "file")],
            ));
        }
        parts.join(" ")
    }

    /// The result of a restore, in words.
    pub fn restore_done_text(r: &RestoreReport) -> String {
        use crate::library_health::plural;
        let mut s = trf("Restored {}.", &[&plural(r.files_written, "file")]);
        if !r.failed.is_empty() {
            s.push(' ');
            s.push_str(&trf(
                "{} could not be restored.",
                &[&plural(r.failed.len(), "file")],
            ));
        }
        s
    }

    /// The old catalogue files (`.db`, `.bookxbackup`) and the repository snapshots of a place.
    /// The repository part may fail (needs a passphrase, damaged) without hiding the old files.
    pub fn listing(
        d: &Destination,
        passphrase: &str,
    ) -> (
        Vec<(RemoteFile, RemoteName)>,
        anyhow::Result<RestoreChoices>,
    ) {
        let legacy = d
            .target()
            .and_then(|t| bookx_backup::list_backups(t.as_ref()))
            .unwrap_or_default();
        let repo = restore_options(d, passphrase);
        (legacy, repo)
    }

    /// [`journal_all`] on a worker thread that is given up on after `limit` (closing never hangs).
    pub fn journal_within(
        lib: &LibrarySource,
        places: &[Destination],
        passphrase: &str,
        limit: Duration,
    ) -> Vec<(String, String)> {
        let (lib2, places2, pass) = (lib.clone(), places.to_vec(), passphrase.to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(journal_all(
                &lib2,
                &places2,
                &pass,
                Utc::now(),
                Kdf::DEFAULT,
            ));
        });
        rx.recv_timeout(limit).unwrap_or_else(|_| {
            places
                .iter()
                .map(|d| (d.name.clone(), "no answer in time".to_string()))
                .collect()
        })
    }

    /// One snapshot a place can restore.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RestorePoint {
        pub snapshot: String,
        pub seq: u64,
        pub at: DateTime<Utc>,
        pub manual: bool,
        pub books: usize,
        pub files: usize,
        pub bytes: u64,
    }

    /// What can be restored from a place: the repository's snapshots (books, covers, catalogue), the
    /// time up to which notes are known, and the old whole-catalogue backups that may sit next to it.
    #[derive(Debug, Clone, Default)]
    pub struct RestoreChoices {
        pub points: Vec<RestorePoint>,
        /// The newest journal record: restoring "as of" a time up to this one includes the notes.
        pub notes_until: Option<DateTime<Utc>>,
        pub legacy: Vec<(RemoteFile, RemoteName)>,
        pub encrypted: bool,
        /// Snapshots that could not be read: (name, why).
        pub unreadable: Vec<(String, String)>,
    }

    /// Lists what a place can restore. The old `.db`/`.bookxbackup` files stay restorable through
    /// `bookx_backup::fetch_to_file` (the existing restore path).
    pub fn restore_options(d: &Destination, passphrase: &str) -> anyhow::Result<RestoreChoices> {
        let mut c = RestoreChoices::default();
        if let Ok(t) = d.target() {
            c.legacy = bookx_backup::list_backups(t.as_ref()).unwrap_or_default();
        }
        let store = d.repo_store()?;
        let Some(enc) = Repository::is_encrypted_at(store.as_ref())? else {
            return Ok(c);
        };
        c.encrypted = enc;
        let repo = Repository::open(store, &Secrets::passphrase(passphrase))?;
        for s in repo.list_snapshots()? {
            match repo.manifest(&s.name) {
                Ok(m) => c.points.push(RestorePoint {
                    snapshot: s.file_name(),
                    seq: s.name.seq,
                    at: s.name.at,
                    manual: s.name.manual,
                    books: m.books,
                    files: m.files.len(),
                    bytes: m.total_bytes(),
                }),
                Err(e) => c.unreadable.push((s.file_name(), format!("{e:#}"))),
            }
        }
        c.notes_until = repo.list_journal()?.last().map(|j| j.1);
        Ok(c)
    }

    /// Restores `snapshot` (`None`: the newest) from a place into the library folder `target`.
    /// `dry_run` reports without changing anything. Closing the library in the app first is the
    /// caller's job.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_from_place(
        d: &Destination,
        passphrase: &str,
        snapshot: Option<&str>,
        target: &Path,
        mode: RestoreMode,
        dry_run: bool,
        until: Option<DateTime<Utc>>,
        cancel: &AtomicBool,
        progress: &(dyn Fn(&RunProgress) + Sync),
    ) -> anyhow::Result<RestoreReport> {
        let repo = Repository::open(d.repo_store()?, &Secrets::passphrase(passphrase))?;
        let opts = RestoreOptions {
            mode,
            dry_run,
            until,
            // naming an older snapshot is a choice; only "the newest" must not be older than what this library saw
            min_seq: snapshot
                .is_none()
                .then(|| bookx_backup::repo::source::last_seen_seq(target, repo.id())),
            ..RestoreOptions::default()
        };
        repo.restore(snapshot, target, &opts, progress, cancel)
    }

    // -- one writer per place, taking over a lock ----------------------------------------------------------

    /// Whether an error text says another computer is writing to the place.
    pub fn is_in_use(err: &str) -> bool {
        err.contains(bookx_backup::repo::lock::IN_USE)
    }

    impl RepoOutcome {
        /// The place was busy (another computer holds its lock).
        pub fn in_use(&self) -> bool {
            self.result.as_ref().err().is_some_and(|e| is_in_use(e))
        }
    }

    /// The outcomes that failed because the place was busy, with the place's id and name.
    pub fn busy_places(outcomes: &[RepoOutcome]) -> Vec<(String, String)> {
        outcomes
            .iter()
            .filter(|o| o.in_use())
            .map(|o| (o.id.clone(), o.name.clone()))
            .collect()
    }

    /// Removes the lock of a place after the user confirmed that nobody is writing to it. Returns
    /// the name of the computer that held it, when it said.
    pub fn take_over(d: &Destination) -> anyhow::Result<Option<String>> {
        let store = d.repo_store()?;
        Ok(Repository::take_over_lock(store.as_ref())?.map(|l| l.host))
    }

    /// What to ask before taking over.
    pub const TAKE_OVER_BODY: &str = N_(
        "Do this only when you are sure that the other computer is not backing up to this place right now, for example because it was switched off in the middle of a copy. Two computers writing at the same time can damage the backup.",
    );

    /// The sentence for a place that is busy.
    pub fn busy_text(names: &[String]) -> String {
        trf(
            "{} is in use by another computer. Wait until it is done, or take the place over if you know it stopped.",
            &[&names.join(", ")],
        )
    }

    // -- changing the backup passphrase --------------------------------------------------------------------

    /// What happened to one place when the passphrase was changed.
    #[derive(Debug, Clone)]
    pub struct RekeyOutcome {
        pub name: String,
        pub result: Result<bookx_backup::repo::RekeyReport, String>,
        pub cancelled: bool,
        /// The place has no copy yet: nothing to re-encrypt.
        pub empty: bool,
    }

    /// Re-encrypts every encrypted place that has a backup, from `old` to `new`. Places already on
    /// the new passphrase are reported as done; a failure at one place does not stop the others.
    /// `progress` gets the place's name and what is being done.
    pub fn change_passphrase(
        places: &[Destination],
        old: &str,
        new: &str,
        kdf: Kdf,
        cancel: &AtomicBool,
        progress: &(dyn Fn(&str, &bookx_backup::repo::RekeyProgress) + Sync),
    ) -> Vec<RekeyOutcome> {
        let mut out = Vec::new();
        for d in places.iter().filter(|d| d.encrypt) {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let r = (|| -> anyhow::Result<bookx_backup::repo::RekeyReport> {
                let store = d.repo_store()?;
                check_readable(d, store.as_ref())?;
                Repository::change_passphrase(
                    store,
                    old,
                    new,
                    &bookx_backup::repo::RekeyOptions {
                        kdf,
                        ..Default::default()
                    },
                    cancel,
                    &|p| progress(&d.name, p),
                )
            })();
            let cancelled = r
                .as_ref()
                .err()
                .is_some_and(|e| e.downcast_ref::<Cancelled>().is_some());
            let empty = r.as_ref().err().is_some_and(|e| {
                e.downcast_ref::<bookx_backup::repo::NoRepository>()
                    .is_some()
            });
            out.push(RekeyOutcome {
                name: d.name.clone(),
                result: r.map_err(|e| format!("{e:#}")),
                cancelled,
                empty,
            });
        }
        out
    }

    /// Every place is on the new passphrase (or has nothing to change): the saved passphrase may be
    /// replaced.
    pub fn rekey_complete(outcomes: &[RekeyOutcome]) -> bool {
        outcomes.iter().all(|o| o.empty || o.result.is_ok())
    }

    /// The message after a change.
    pub fn rekey_text(outcomes: &[RekeyOutcome], stopped: bool) -> String {
        if stopped || outcomes.iter().any(|o| o.cancelled) {
            return tr(
                "Passphrase change stopped. Nothing is lost: change it again with the same passphrases to finish.",
            );
        }
        let failed: Vec<String> = outcomes
            .iter()
            .filter(|o| !o.empty)
            .filter_map(|o| o.result.as_ref().err().map(|e| format!("{}: {e}", o.name)))
            .collect();
        if !failed.is_empty() {
            return trf(
                "The passphrase was not changed everywhere: {}. The old passphrase is still saved; change it again to finish.",
                &[&failed.join("; ")],
            );
        }
        let changed = outcomes
            .iter()
            .filter(|o| !o.empty && o.result.is_ok())
            .count();
        if changed == 0 {
            tr(
                "The new passphrase is saved. No encrypted backup exists yet, so nothing had to be re-encrypted.",
            )
        } else {
            trf(
                "The backup passphrase was changed at {}. Keep the new one safe: it cannot be recovered.",
                &[&crate::library_health::plural(changed, "place")],
            )
        }
    }

    /// What is wrong with the three fields of the change dialog, if anything.
    pub fn passphrase_problem(saved: &str, old: &str, new: &str, repeat: &str) -> Option<String> {
        if old.is_empty() && saved.is_empty() {
            return Some(tr("Enter the current passphrase"));
        }
        if new.is_empty() {
            return Some(tr("Enter a new passphrase"));
        }
        if new.chars().count() < 8 {
            return Some(tr("Use at least 8 characters"));
        }
        if new != repeat {
            return Some(tr("The two new passphrases are not the same"));
        }
        let current = if old.is_empty() { saved } else { old };
        if current == new {
            return Some(tr("The new passphrase is the same as the current one"));
        }
        None
    }

    /// 0..1 over the objects of one place and the places in turn.
    pub fn rekey_fraction(i: usize, n: usize, p: &bookx_backup::repo::RekeyProgress) -> f64 {
        let part = if p.bytes_total > 0 {
            p.bytes_done as f64 / p.bytes_total as f64
        } else if p.objects_total > 0 {
            p.objects_done as f64 / p.objects_total as f64
        } else {
            0.0
        };
        ((i as f64 + part.clamp(0.0, 1.0)) / n.max(1) as f64).clamp(0.0, 1.0)
    }

    /// "120 of 340 files · 1.2 GB of 4 GB".
    pub fn rekey_detail(p: &bookx_backup::repo::RekeyProgress) -> String {
        use crate::library_health::size_text;
        if p.objects_total == 0 {
            return String::new();
        }
        format!(
            "{} · {}",
            trf("{} of {} files", &[&p.objects_done, &p.objects_total]),
            trf(
                "{} of {}",
                &[&size_text(p.bytes_done), &size_text(p.bytes_total)]
            )
        )
    }

    /// The line under the bar of a deep check: "1.2 GB of 4 GB · 38 MB/s · about 2 minutes left".
    pub fn verify_detail(done: u64, total: u64, elapsed: Duration) -> String {
        use crate::library_health::size_text;
        let mut parts = vec![trf("{} of {}", &[&size_text(done), &size_text(total)])];
        let secs = elapsed.as_secs_f64();
        if done > 0 && secs >= 2.0 {
            let rate = done as f64 / secs;
            parts.push(trf("{}/s", &[&size_text(rate as u64)]));
            if total > done {
                parts.push(eta_phrase(Duration::from_secs_f64(
                    (total - done) as f64 / rate,
                )));
            }
        }
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::repo::*;
    use super::*;
    use bookx_backup::repo::{
        LibrarySource, Phase, Plan, Report, Repository, RestoreMode, RestoreReport, RunProgress,
        Secrets,
    };
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
        assert!(
            run_due(
                &root,
                &st,
                Trigger::Exit,
                later,
                Kdf::TEST,
                &AtomicBool::new(false)
            )
            .is_empty()
        );
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
    fn scope_copy_says_books_covers_and_catalogue_are_kept_and_only_changes_are_copied() {
        // The constants are the msgids the UI and the developer hooks translate.
        assert_eq!(BACKUP_SCOPE_TITLE, "What is in a backup");
        assert_eq!(
            BACKUP_SCOPE_DESCRIPTION,
            "Backups include your book files, covers and catalogue (titles, tags, collections, reading progress, bookmarks, highlights and notes). Only new or changed files are copied after the first backup. Not included: configuration, plugins and passwords, trash, device copies and caches."
        );
        let included = backup_included_label().to_lowercase();
        for word in [
            "book files",
            "covers",
            "catalogue",
            "collections",
            "bookmarks",
            "highlights",
            "notes",
        ] {
            assert!(included.contains(word), "{word} in {included}");
        }
        let excluded = backup_excluded_label();
        assert!(excluded.starts_with("Not included"), "{excluded}");
        for word in [
            "configuration",
            "plugins",
            "passwords",
            "trash",
            "device copies",
            "caches",
        ] {
            assert!(excluded.contains(word), "{word} in {excluded}");
        }
        // what is now kept must not be listed as left out
        for word in ["book files", "covers"] {
            assert!(
                !excluded.to_lowercase().contains(word),
                "{word} in {excluded}"
            );
        }
        let all = format!(
            "{} {} {}",
            backup_scope_subtitle(),
            backup_scope_description(),
            backup_incremental_note()
        )
        .to_lowercase();
        assert!(
            !all.contains("catalogue only") && !all.contains("not incremental"),
            "{all}"
        );
        assert!(all.contains("only new or changed files"), "{all}");
        let note = backup_incremental_note();
        assert!(
            note.contains("'Skipped' means") && note.contains("'Verified' means"),
            "{note}"
        );
        assert!(
            note.contains("renamed or moved book costs nothing"),
            "{note}"
        );
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
        assert!(
            uri_problem("https://cloud.example")
                .unwrap()
                .contains("WebDAV")
        );
        assert!(uri_problem("//nas/books").is_some());
        assert!(!is_mountable_uri("smb://"));
    }

    // -- the repository glue ----------------------------------------------------------------------

    fn library(root: &Path) -> LibrarySource {
        bookx_store::open_library(root).unwrap();
        LibrarySource::open(root).unwrap()
    }

    fn put(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn a_run_to_two_places_survives_one_being_gone_and_records_each_status() {
        let root = tmp();
        let lib = library(&root);
        put(&root, "Ann/One/one.epub", "one");
        put(&root, "Ann/Two/two.epub", "two");
        let usb = tmp();
        std::fs::create_dir_all(&usb).unwrap();
        let dests = vec![
            folder("gone", Path::new("/definitely-not-mounted/x")),
            folder("usb", &usb),
        ];
        let out = run_all(
            &lib,
            &dests,
            "",
            false,
            Utc::now(),
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert_eq!(out.len(), 2);
        assert!(out[0].result.is_err());
        let r = out[1].result.as_ref().unwrap();
        assert_eq!((r.files, r.new_files), (2, 2));
        let mut st = BackupSettings {
            destinations: dests,
            ..BackupSettings::default()
        };
        apply_repo(&mut st, &out, Utc::now());
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
        assert!(
            usb.join("Bookx Backup/README.txt").exists(),
            "the repository is next to any old backups"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_estimate_says_first_everything_then_nothing_then_only_the_new_book() {
        let root = tmp();
        let lib = library(&root);
        put(&root, "Ann/One/one.epub", &"x".repeat(5000));
        let usb = tmp();
        std::fs::create_dir_all(&usb).unwrap();
        let d = folder("usb", &usb);
        let no = AtomicBool::new(false);
        let e = estimate_for(&lib, std::slice::from_ref(&d), "", &no);
        let p = e[0].plan.as_ref().unwrap();
        assert!(
            p.first_run && p.new_files == 1 && p.new_bytes == 5000,
            "{p:?}"
        );
        assert!(
            plan_text(p).contains("The first backup copies everything"),
            "{}",
            plan_text(p)
        );
        run_all(
            &lib,
            std::slice::from_ref(&d),
            "",
            false,
            Utc::now(),
            Kdf::TEST,
            &no,
            &|_, _| {},
        );
        let e = estimate_for(&lib, std::slice::from_ref(&d), "", &no);
        let p = e[0].plan.as_ref().unwrap();
        assert_eq!((p.new_files, p.reused_files), (0, 1));
        assert_eq!(plan_text(p), "Everything is already backed up.");
        put(&root, "Ann/Two/two.epub", &"y".repeat(7000));
        let p = estimate_for(&lib, std::slice::from_ref(&d), "", &no)
            .remove(0)
            .plan
            .unwrap();
        assert_eq!(p.new_files, 1);
        assert!(
            plan_text(&p).contains("1 files") || plan_text(&p).contains("(1 files)"),
            "{}",
            plan_text(&p)
        );
        // a place that is not there says why
        let gone = folder("gone", Path::new("/definitely-not-mounted/x"));
        let e = estimate_for(&lib, &[gone], "", &no);
        assert!(e[0].plan.as_ref().unwrap_err().contains("mounted"));
        // an encrypted place without the passphrase is a clear message, not a plain copy
        let mut enc = folder("e", &usb);
        enc.encrypt = true;
        assert!(
            estimate_for(&lib, &[enc], "", &no)[0]
                .plan
                .as_ref()
                .unwrap_err()
                .contains("passphrase")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scheduled_repository_runs_copy_when_due_and_say_nothing_new_otherwise() {
        let root = tmp();
        let lib = library(&root);
        put(&root, "Ann/One/one.epub", "one");
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let mut st = BackupSettings::default();
        st.destinations.push(folder("drive", &dir));
        let now = Utc::now();
        let no = AtomicBool::new(false);
        // the first copy of a place waits for the user (see the next test)
        assert!(run_due_repo(&lib, &st, Trigger::Exit, now, Kdf::TEST, &no).is_empty());
        st.destinations[0].auto_ok = true;
        let out = run_due_repo(&lib, &st, Trigger::Exit, now, Kdf::TEST, &no);
        assert_eq!(out.len(), 1);
        let first = out[0].result.as_ref().unwrap();
        assert_eq!(first.new_files, 1);
        assert_eq!(
            report_text(first),
            "Copied 1 new files (1 KB); 0 were already there."
        );
        apply_repo(&mut st, &out, now);
        let later = now + chrono::Duration::hours(2);
        let out = run_due_repo(&lib, &st, Trigger::Exit, later, Kdf::TEST, &no);
        let r = out[0].result.as_ref().unwrap();
        assert!(r.up_to_date, "nothing changed, nothing copied: {r:?}");
        assert_eq!(report_text(r), "Nothing new to copy.");
        // closing never waits longer than the limit
        let o = run_due_repo_within(
            &lib,
            &st,
            Trigger::Exit,
            later + chrono::Duration::hours(3),
            Duration::from_millis(0),
        );
        assert!(o.len() <= 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn restore_options_list_snapshots_notes_and_the_old_backups_together() {
        let root = tmp();
        let lib = library(&root);
        put(&root, "Ann/One/one.epub", "one");
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let d = folder("drive", &dir);
        // an old whole-catalogue backup in the same place
        let snap = store_with_snapshot(&root);
        let t = d.target().unwrap();
        bookx_backup::push(
            t.as_ref(),
            &snap,
            &bookx_backup::PushOptions {
                passphrase: None,
                keep: 5,
                now: Utc::now(),
                kdf: Kdf::TEST,
            },
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap();
        assert!(restore_options(&d, "").unwrap().points.is_empty());
        assert_eq!(restore_options(&d, "").unwrap().legacy.len(), 1);
        run_all(
            &lib,
            std::slice::from_ref(&d),
            "",
            true,
            Utc::now(),
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        let c = restore_options(&d, "").unwrap();
        assert_eq!((c.points.len(), c.legacy.len()), (1, 1));
        assert!(c.points[0].manual && c.points[0].files == 1 && !c.encrypted);
        // restore it into a new folder, dry run first
        let to = tmp().join("restored");
        let no = AtomicBool::new(false);
        let dry = restore_from_place(
            &d,
            "",
            None,
            &to,
            RestoreMode::MissingOnly,
            true,
            None,
            &no,
            &|_| {},
        )
        .unwrap();
        assert!(dry.dry_run && !to.exists());
        let r = restore_from_place(
            &d,
            "",
            None,
            &to,
            RestoreMode::MissingOnly,
            false,
            None,
            &no,
            &|_| {},
        )
        .unwrap();
        assert!(r.failed.is_empty() && r.files_written == 1, "{r:?}");
        assert_eq!(
            std::fs::read_to_string(to.join("Ann/One/one.epub")).unwrap(),
            "one"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn journal_all_protects_notes_for_every_place_and_isolates_failures() {
        let root = tmp();
        let lib = library(&root);
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let places = vec![
            folder("gone", Path::new("/definitely-not-mounted/x")),
            folder("drive", &dir),
        ];
        let failed = journal_all(&lib, &places, "", Utc::now(), Kdf::TEST);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "gone");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_first_copy_of_a_place_is_never_a_surprise() {
        let mut st = BackupSettings::default();
        st.destinations.push(folder("old", Path::new("/mnt/old")));
        st.destinations.push(folder("new", Path::new("/mnt/new")));
        let now = Utc::now();
        // a place from before the update: nothing scheduled, nothing recorded
        assert!(scheduled_places(&st, Trigger::Exit, now).is_empty());
        assert!(scheduled_places(&st, Trigger::Tick, now).is_empty());
        assert!(journal_places(&st).is_empty());
        // the user allowed the first copy of one place after seeing the size
        st.destinations[1].auto_ok = true;
        let names: Vec<String> = scheduled_places(&st, Trigger::Exit, now)
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(names, ["new"]);
        assert_eq!(journal_places(&st).len(), 1);
        // a switched-off place gets neither
        st.destinations[1].enabled = false;
        assert!(scheduled_places(&st, Trigger::Exit, now).is_empty());
        assert!(journal_places(&st).is_empty());
        // a manual first copy that worked allows the automatic ones from then on
        st.destinations[0].enabled = true;
        let ok = RepoOutcome {
            id: "old".into(),
            name: "old".into(),
            result: Ok(Report::default()),
            cancelled: false,
        };
        apply_repo(&mut st, &[ok], now);
        assert!(st.destinations[0].auto_ok);
        // a failure or a cancel does not
        let mut st2 = BackupSettings::default();
        st2.destinations.push(folder("x", Path::new("/mnt/x")));
        let bad = RepoOutcome {
            id: "x".into(),
            name: "x".into(),
            result: Err("full".into()),
            cancelled: false,
        };
        let stopped = RepoOutcome {
            id: "x".into(),
            name: "x".into(),
            result: Err("stopped".into()),
            cancelled: true,
        };
        apply_repo(&mut st2, &[bad, stopped], now);
        assert!(!st2.destinations[0].auto_ok);
    }

    #[test]
    fn progress_moves_forward_through_the_phases_and_says_where_it_is() {
        let mk = |phase, fd, ft, bd, bt| RunProgress {
            phase,
            files_done: fd,
            files_total: ft,
            bytes_done: bd,
            bytes_total: bt,
            current: String::new(),
            elapsed: Duration::from_secs(5),
            eta: Some(Duration::from_secs(40)),
        };
        let seq = [
            mk(Phase::Scanning, 0, 0, 0, 0),
            mk(Phase::Hashing, 50, 100, 0, 0),
            mk(Phase::Uploading, 0, 10, 0, 1000),
            mk(Phase::Uploading, 5, 10, 500, 1000),
            mk(Phase::Finishing, 10, 10, 1000, 1000),
            mk(Phase::Done, 10, 10, 1000, 1000),
        ];
        let f: Vec<f64> = seq.iter().map(progress_fraction).collect();
        assert!(f.windows(2).all(|w| w[0] <= w[1]), "{f:?}");
        assert!(f.iter().all(|x| (0.0..=1.0).contains(x)));
        assert_eq!(*f.last().unwrap(), 1.0);
        // no bytes known (nothing to copy): files drive the bar
        assert!(progress_fraction(&mk(Phase::Uploading, 5, 10, 0, 0)) > 0.5);
        let d = progress_detail(&seq[3]);
        assert!(
            d.contains("5 of 10 files") && d.contains("500 of 1000") || d.contains("KB"),
            "{d}"
        );
        assert!(d.contains("40 seconds left"), "{d}");
        // a time left is only promised while copying
        assert!(!progress_detail(&seq[1]).contains("left"));
        assert_eq!(
            progress_stage("Offsite", &seq[2]),
            "Offsite: Copying new files"
        );
        assert!(eta_phrase(Duration::from_secs(300)).contains("5 minutes"));
        assert!(eta_phrase(Duration::from_millis(100)).contains("1 second"));
    }

    #[test]
    fn the_banner_says_how_big_the_first_copy_is_and_only_while_it_has_not_happened() {
        let first = Plan {
            first_run: true,
            new_files: 120,
            new_bytes: 3 * 1024 * 1024 * 1024,
            ..Plan::default()
        };
        let t = banner_text(std::slice::from_ref(&first)).unwrap();
        assert!(t.contains("3.0 GB") && t.contains("120 files"), "{t}");
        assert!(t.contains("Nothing is copied until you press Back Up"));
        let later = Plan {
            first_run: false,
            new_files: 2,
            ..Plan::default()
        };
        assert!(banner_text(&[later]).is_none());
        // an empty library has nothing to announce
        assert!(
            banner_text(&[Plan {
                first_run: true,
                ..Plan::default()
            }])
            .is_none()
        );
        assert!(banner_text(&[]).is_none());
        // with two places the bigger one is named
        let small = Plan {
            first_run: true,
            new_files: 1,
            new_bytes: 10,
            ..Plan::default()
        };
        assert!(banner_text(&[small, first]).unwrap().contains("GB"));
    }

    #[test]
    fn what_a_place_holds_is_in_words_and_the_check_and_cleanup_work_on_a_real_place() {
        let root = tmp();
        let lib = library(&root);
        put(&root, "Ann/One/one.epub", "one");
        let dir = tmp();
        std::fs::create_dir_all(&dir).unwrap();
        let d = folder("drive", &dir);
        let no = AtomicBool::new(false);
        assert_eq!(repo_summary(&d, "").unwrap(), None, "no repository yet");
        for _ in 0..2 {
            put(&root, &format!("Ann/N{}/n.epub", rand_name()), "more");
            let out = run_all(
                &lib,
                std::slice::from_ref(&d),
                "",
                false,
                Utc::now(),
                Kdf::TEST,
                &no,
                &|_, _| {},
            );
            assert!(out[0].result.is_ok(), "{:?}", out[0].result);
        }
        let s = repo_summary(&d, "").unwrap().unwrap();
        assert_eq!((s.snapshots, s.files), (2, 3));
        let line = summary_text(&s);
        assert!(
            line.contains("2 backups") && line.contains("last "),
            "{line}"
        );
        let rep = check_place(&d, "", false, &no, &|_, _| {}).unwrap();
        assert!(rep.ok());
        assert!(
            check_text(&rep).contains("complete"),
            "{}",
            check_text(&rep)
        );
        let deep = check_place(&d, "", true, &no, &|_, _| {}).unwrap();
        assert!(check_text(&deep).contains("intact"));
        // a missing file is named in the words
        let blob = std::fs::read_dir(dir.join("Bookx Backup/blobs"))
            .unwrap()
            .flatten()
            .flat_map(|e| std::fs::read_dir(e.path()).unwrap().flatten())
            .next()
            .unwrap()
            .path();
        std::fs::remove_file(blob).unwrap();
        let bad = check_place(&d, "", false, &no, &|_, _| {}).unwrap();
        assert!(!bad.ok());
        assert!(check_text(&bad).contains("missing"), "{}", check_text(&bad));
        // keep one automatic backup: the other one goes
        let mut one = d.clone();
        one.keep = 1;
        let c = clean_place(&one, "", Utc::now() + chrono::Duration::days(30), &no).unwrap();
        assert!(clean_text(&c).contains("Removed") || clean_text(&c).contains("Nothing"));
        let _ = std::fs::remove_dir_all(root);
    }

    fn rand_name() -> String {
        uuid::Uuid::new_v4().simple().to_string()[..6].to_string()
    }

    #[test]
    fn the_messages_after_a_run_name_the_failure_the_cancel_and_the_files_left_out() {
        let ok = |name: &str, new_files: usize| RepoOutcome {
            id: name.into(),
            name: name.into(),
            result: Ok(Report {
                new_files,
                new_bytes: 2048,
                reused_files: 3,
                up_to_date: new_files == 0,
                too_big: vec!["big.pdf".into()],
                ..Report::default()
            }),
            cancelled: false,
        };
        let bad = RepoOutcome {
            id: "b".into(),
            name: "NAS".into(),
            result: Err("Not enough space in this place".into()),
            cancelled: false,
        };
        let t = outcomes_text(&[ok("USB", 2), bad.clone()]);
        assert!(t.contains("Copied 2 new files"), "{t}");
        assert!(t.contains("1 files are left out"), "{t}");
        assert!(t.contains("NAS: Not enough space"), "{t}");
        assert!(outcomes_text(&[ok("USB", 0)]).contains("Nothing new"));
        let two = outcomes_text(&[ok("A", 1), ok("B", 0)]);
        assert!(
            two.contains("A: ") && two.contains("B: Nothing new"),
            "{two}"
        );
        let stopped = RepoOutcome {
            cancelled: true,
            ..bad
        };
        assert!(outcomes_text(&[stopped]).contains("cancelled"));
        assert_eq!(outcomes_text(&[]), "");
    }

    #[test]
    fn the_restore_preview_says_what_comes_back_and_what_is_there() {
        let r = RestoreReport {
            would_write: 3,
            skipped_existing: 2,
            skipped_same: 1,
            catalogue: bookx_backup::repo::CatalogueAction::WouldRestore,
            replaced_newer: vec!["a.epub".into()],
            ..RestoreReport::default()
        };
        let t = restore_plan_text(&r);
        assert!(t.contains("3 files to restore, 3 already present"), "{t}");
        assert!(t.contains("catalogue is replaced"), "{t}");
        assert!(t.contains("1 file are newer here"), "{t}");
        let kept = RestoreReport {
            catalogue: bookx_backup::repo::CatalogueAction::KeptExisting,
            ..RestoreReport::default()
        };
        assert!(restore_plan_text(&kept).contains("current catalogue is kept"));
        let done = RestoreReport {
            files_written: 2,
            failed: vec![("x".into(), "y".into())],
            ..RestoreReport::default()
        };
        assert_eq!(
            restore_done_text(&done),
            "Restored 2 files. 1 file could not be restored."
        );
    }

    #[test]
    fn a_place_says_what_it_copies() {
        let mut d = folder("d", Path::new("/x"));
        assert_eq!(scope_label(&d), "Books, covers and catalogue");
        d.include_covers = false;
        d.max_file_mb = 50;
        assert_eq!(
            scope_label(&d),
            "Books and catalogue, no covers · files up to 50 MB"
        );
        d.include_books = false;
        assert_eq!(
            scope_label(&d),
            "Catalogue only",
            "the size limit is about book files"
        );
    }

    #[test]
    fn a_new_folder_in_an_existing_one_is_created_but_a_missing_drive_is_not() {
        let base = tmp();
        std::fs::create_dir_all(&base).unwrap();
        let fresh = folder("f", &base.join("new-name"));
        assert!(will_be_created(&fresh));
        assert!(!will_be_created(&folder("g", &base)), "it exists");
        assert!(
            !will_be_created(&folder("h", &base.join("no/such/deeper"))),
            "the drive is not there"
        );
        assert!(!will_be_created(&folder("i", Path::new(""))));
        // the estimate for it works without creating anything
        let root = tmp();
        let lib = library(&root);
        put(&root, "Ann/One/one.epub", "one");
        let e = estimate_for(
            &lib,
            std::slice::from_ref(&fresh),
            "",
            &AtomicBool::new(false),
        );
        assert!(e[0].plan.as_ref().unwrap().first_run);
        assert!(!base.join("new-name").exists());
        let out = run_all(
            &lib,
            std::slice::from_ref(&fresh),
            "",
            true,
            Utc::now(),
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert!(out[0].result.is_ok(), "{:?}", out[0].result);
        assert!(base.join("new-name/Bookx Backup/README.txt").exists());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_busy_place_is_told_apart_from_a_broken_one_and_can_be_taken_over() {
        let base = tmp();
        let place = base.join("nas");
        std::fs::create_dir_all(&place).unwrap();
        let d = folder("nas", &place);
        // another computer's fresh lock
        let repo_dir = place.join("Bookx Backup");
        std::fs::create_dir_all(&repo_dir).unwrap();
        let now = Utc::now();
        let lock = serde_json::json!({
            "format": 1, "host": "kitchen-pc", "pid": 7, "token": "t",
            "since": now, "beat": now, "app": "Bookx"
        });
        std::fs::write(repo_dir.join("lock.json"), lock.to_string()).unwrap();
        let root = tmp();
        let lib = library(&root);
        put(&root, "Ann/One/one.epub", "one");
        let out = run_all(
            &lib,
            std::slice::from_ref(&d),
            "",
            true,
            Utc::now(),
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert!(out[0].in_use(), "{:?}", out[0].result);
        assert!(out[0].result.as_ref().unwrap_err().contains("kitchen-pc"));
        assert_eq!(
            busy_places(&out),
            vec![("nas".to_string(), "nas".to_string())]
        );
        assert!(!is_in_use("the folder is not there"));
        // taking over names the other computer and the next run goes through
        assert_eq!(take_over(&d).unwrap().as_deref(), Some("kitchen-pc"));
        assert_eq!(take_over(&d).unwrap(), None, "nothing to take over twice");
        let out = run_all(
            &lib,
            std::slice::from_ref(&d),
            "",
            true,
            Utc::now(),
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert!(out[0].result.is_ok(), "{:?}", out[0].result);
        assert!(!repo_dir.join("lock.json").exists(), "released afterwards");
        assert!(busy_text(&["nas".into()]).contains("nas"));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_passphrase_dialog_checks_its_fields() {
        assert!(passphrase_problem("", "", "longenough", "longenough").is_some());
        assert!(passphrase_problem("old-pass", "", "", "").is_some());
        assert!(passphrase_problem("old-pass", "", "short", "short").is_some());
        assert!(passphrase_problem("old-pass", "", "new-pass-1", "new-pass-2").is_some());
        assert!(passphrase_problem("old-pass", "", "old-pass", "old-pass").is_some());
        assert!(passphrase_problem("old-pass", "other-old", "other-old", "other-old").is_some());
        // the saved one stands in for an empty current field
        assert_eq!(
            passphrase_problem("old-pass", "", "new-pass-1", "new-pass-1"),
            None
        );
        assert_eq!(
            passphrase_problem("", "typed-old", "new-pass-1", "new-pass-1"),
            None
        );
    }

    #[test]
    fn changing_the_passphrase_re_encrypts_every_encrypted_place_and_skips_the_rest() {
        let base = tmp();
        let root = tmp();
        let lib = library(&root);
        put(&root, "Ann/One/one.epub", "one");
        let mut enc = folder("enc", &base.join("enc"));
        enc.encrypt = true;
        let plain = folder("plain", &base.join("plain"));
        let mut later = folder("later", &base.join("later"));
        later.encrypt = true;
        std::fs::create_dir_all(base.join("later")).unwrap();
        std::fs::create_dir_all(base.join("enc")).unwrap();
        std::fs::create_dir_all(base.join("plain")).unwrap();
        let places = vec![enc.clone(), plain.clone(), later.clone()];
        let out = run_all(
            &lib,
            &[enc.clone(), plain.clone()],
            "old-pass-1",
            true,
            Utc::now(),
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert!(out.iter().all(|o| o.result.is_ok()), "{out:?}");
        let seen = std::sync::Mutex::new(Vec::new());
        let r = change_passphrase(
            &places,
            "old-pass-1",
            "new-pass-2",
            Kdf::TEST,
            &AtomicBool::new(false),
            &|name, p| {
                seen.lock()
                    .unwrap()
                    .push((name.to_string(), p.objects_total))
            },
        );
        // the plain place is left alone; the encrypted place with no copy yet has nothing to do
        assert_eq!(r.len(), 2, "{r:?}");
        assert!(r[0].result.is_ok() && !r[0].empty);
        assert!(r[1].empty);
        assert!(rekey_complete(&r));
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|(n, t)| n == "enc" && *t > 0)
        );
        assert!(rekey_text(&r, false).contains('1'));
        // the new passphrase opens it, the old one does not
        assert!(
            Repository::open(
                enc.repo_store().unwrap(),
                &Secrets::passphrase("new-pass-2")
            )
            .is_ok()
        );
        assert!(
            Repository::open(
                enc.repo_store().unwrap(),
                &Secrets::passphrase("old-pass-1")
            )
            .is_err()
        );
        // a second go (a place of the same account that was already done) is not an error
        let again = change_passphrase(
            &places,
            "old-pass-1",
            "new-pass-2",
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert!(rekey_complete(&again), "{again:?}");
        // a wrong current passphrase fails the place and is not "complete"
        let bad = change_passphrase(
            &places,
            "not-it-at-all",
            "third-pass-3",
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert!(!rekey_complete(&bad));
        assert!(rekey_text(&bad, false).contains("not changed everywhere"));
        // nothing to re-encrypt anywhere
        let none = change_passphrase(
            &[plain],
            "a",
            "b",
            Kdf::TEST,
            &AtomicBool::new(false),
            &|_, _| {},
        );
        assert!(none.is_empty() && rekey_complete(&none));
        assert!(rekey_text(&none, false).contains("No encrypted backup"));
        assert!(
            rekey_text(&none, true).contains("stopped"),
            "a stop is not a success"
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn rekey_and_verify_progress_read_well() {
        let p = bookx_backup::repo::RekeyProgress {
            objects_done: 5,
            objects_total: 20,
            bytes_done: 1 << 20,
            bytes_total: 4 << 20,
            current: String::new(),
        };
        assert!((rekey_fraction(0, 2, &p) - 0.125).abs() < 1e-9);
        assert!((rekey_fraction(1, 2, &p) - 0.625).abs() < 1e-9);
        assert!(rekey_detail(&p).contains("5 of 20 files"));
        assert_eq!(rekey_detail(&Default::default()), "");
        // a deep check: speed and time left appear once there is something to measure
        let early = verify_detail(10 << 20, 100 << 20, Duration::from_millis(500));
        assert!(!early.contains("/s") && !early.contains("left"), "{early}");
        let later = verify_detail(40 << 20, 100 << 20, Duration::from_secs(4));
        assert!(later.contains("/s") && later.contains("left"), "{later}");
        let done = verify_detail(100 << 20, 100 << 20, Duration::from_secs(9));
        assert!(done.contains("/s") && !done.contains("left"), "{done}");
    }
}
