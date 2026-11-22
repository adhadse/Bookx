//! `bookx-cli backup …`: the incremental backup repository (plan, run, journal, list, restore,
//! verify, gc). A place is a folder (drive, mounted share, cloud-drive mount) or an https WebDAV
//! address; the repository lives in a `Bookx Backup` folder inside it. The passphrase of an
//! encrypted place is in `BOOKX_BACKUP_PASSPHRASE`, a WebDAV password in `BOOKX_WEBDAV_PASSWORD`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{Result, anyhow, bail};
use bookx_backup::crypto::Kdf;
use bookx_backup::repo::store::{REPO_DIR, RepoStore};
use bookx_backup::repo::{
    DavStore, FolderStore, LibrarySource, NoRepository, Repository, RestoreMode, RestoreOptions,
    RunOptions, RunProgress, Scope, Secrets, estimate_new,
};
use chrono::{DateTime, NaiveDate, Utc};
use clap::{Args, Subcommand};

use crate::libtools::RemoteArgs;

#[derive(Args, Debug)]
pub struct PlaceArgs {
    /// The backup place: a folder, or an https:// WebDAV address (password in
    /// BOOKX_WEBDAV_PASSWORD). The repository lives in a "Bookx Backup" folder inside it.
    #[arg(long, alias = "from", value_name = "PLACE")]
    to: String,
    #[command(flatten)]
    remote: RemoteArgs,
}

#[derive(Args, Debug)]
pub struct ScopeArgs {
    /// Leave the book files out (catalogue and covers only)
    #[arg(long)]
    no_books: bool,
    /// Leave the covers out
    #[arg(long)]
    no_covers: bool,
    /// Leave out files bigger than this many megabytes
    #[arg(long, value_name = "MB")]
    max_file_mb: Option<u64>,
}

#[derive(Subcommand, Debug)]
pub enum BackupCmd {
    /// Show what a run would copy (files, bytes, free space) without copying anything
    Plan {
        #[command(flatten)]
        place: PlaceArgs,
        /// The place is (to be) encrypted
        #[arg(long)]
        encrypt: bool,
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long)]
        json: bool,
    },
    /// Back up the library: only new or changed files, the catalogue and the notes are copied
    Run {
        #[command(flatten)]
        place: PlaceArgs,
        /// Encrypt (a new place) with the passphrase in BOOKX_BACKUP_PASSPHRASE
        #[arg(long)]
        encrypt: bool,
        #[command(flatten)]
        scope: ScopeArgs,
        /// How many automatic snapshots the place keeps (manual ones are never removed)
        #[arg(long, default_value_t = 10)]
        keep: usize,
        /// Mark the snapshot manual: it is never pruned
        #[arg(long)]
        manual: bool,
        /// Write a snapshot even when nothing changed
        #[arg(long)]
        force: bool,
        /// Files handled at the same time (1 to 8)
        #[arg(long, default_value_t = 3)]
        jobs: usize,
        /// Go on although the place is older than what this library saw before
        #[arg(long)]
        accept_rollback: bool,
        /// Remove unused blobs after this many hours (0 keeps them until `gc`)
        #[arg(long, default_value_t = 24)]
        gc_hours: u32,
        /// Print progress to stderr
        #[arg(long)]
        progress: bool,
        #[arg(long)]
        json: bool,
    },
    /// Write a small journal record of the notes (highlights, bookmarks, progress) that changed
    /// since the last one
    Journal {
        #[command(flatten)]
        place: PlaceArgs,
        #[arg(long)]
        encrypt: bool,
    },
    /// List the snapshots (and with --journal the journal records) in a place
    List {
        #[command(flatten)]
        place: PlaceArgs,
        #[arg(long)]
        journal: bool,
        #[arg(long)]
        json: bool,
    },
    /// Restore a snapshot (a name, a number, or `latest`) into a folder
    Restore {
        #[arg(default_value = "latest")]
        snapshot: String,
        #[command(flatten)]
        place: PlaceArgs,
        /// The folder to restore into (a new or existing library)
        #[arg(long, value_name = "DIR")]
        into: PathBuf,
        /// missing-only (default) or replace-all
        #[arg(long, default_value = "missing-only")]
        mode: String,
        /// Say what would happen, change nothing
        #[arg(long)]
        dry_run: bool,
        /// Restore the state as of this time (RFC 3339 or YYYY-MM-DD): the snapshot before it and the journal up to it
        #[arg(long)]
        until: Option<String>,
        /// Do not apply the journal after the snapshot
        #[arg(long)]
        no_journal: bool,
        /// Only the catalogue
        #[arg(long)]
        no_files: bool,
        /// Only the files
        #[arg(long)]
        no_catalogue: bool,
        #[arg(long, default_value_t = 3)]
        jobs: usize,
        #[arg(long)]
        json: bool,
    },
    /// Check a place: every snapshot opens and every file it needs is there; --deep reads and
    /// checks every file
    Verify {
        #[command(flatten)]
        place: PlaceArgs,
        #[arg(long)]
        deep: bool,
        /// Remove the damaged files found, so the next run copies them again
        #[arg(long)]
        remove_bad: bool,
        #[arg(long)]
        json: bool,
    },
    /// Remove files no snapshot uses (two steps: noted now, removed after the grace period)
    Gc {
        #[command(flatten)]
        place: PlaceArgs,
        #[arg(long, default_value_t = 24)]
        grace_hours: i64,
        #[arg(long)]
        json: bool,
    },
}

fn store(spec: &str, r: &RemoteArgs) -> Result<Box<dyn RepoStore>> {
    if spec.starts_with("http://") || spec.starts_with("https://") {
        let pw = std::env::var("BOOKX_WEBDAV_PASSWORD").unwrap_or_default();
        return Ok(Box::new(DavStore::new(
            spec,
            r.user.as_deref().unwrap_or(""),
            &pw,
            &r.folder,
        )?));
    }
    if let Some((scheme, _)) = spec.split_once("://") {
        bail!(
            "{scheme}:// cannot be used directly: mount it first (`gio mount {spec}`) and pass the folder under /run/user/<uid>/gvfs/"
        );
    }
    Ok(Box::new(FolderStore::new(Path::new(spec).join(REPO_DIR))))
}

fn secrets() -> Secrets {
    Secrets::passphrase(&std::env::var("BOOKX_BACKUP_PASSPHRASE").unwrap_or_default())
}

fn kdf() -> Kdf {
    // tests may ask for the cheap parameters; the default is the real one
    if crate::var("BOOKX_BACKUP_KDF").is_some_and(|v| v == "test") {
        Kdf::TEST
    } else {
        Kdf::DEFAULT
    }
}

fn open_existing(p: &PlaceArgs) -> Result<Repository> {
    Repository::open(store(&p.to, &p.remote)?, &secrets()).map_err(|e| {
        if e.downcast_ref::<NoRepository>().is_some() {
            anyhow!(
                "there is no Bookx backup in {} (run `bookx-cli backup run --to …` first)",
                p.to
            )
        } else {
            e
        }
    })
}

fn open_or_create(p: &PlaceArgs, encrypt: bool) -> Result<Repository> {
    let st = store(&p.to, &p.remote)?;
    st.check()?;
    if encrypt && secrets().passphrase.is_none() {
        bail!("--encrypt needs the passphrase in BOOKX_BACKUP_PASSPHRASE");
    }
    // an existing repository says whether it is encrypted
    let encrypt = Repository::is_encrypted_at(st.as_ref())?.unwrap_or(encrypt);
    Repository::open_or_create(st, &secrets(), encrypt, kdf())
}

fn scope(s: &ScopeArgs) -> Scope {
    Scope {
        include_books: !s.no_books,
        include_covers: !s.no_covers,
        max_file_size: s.max_file_mb.map(|m| m.saturating_mul(1024 * 1024)),
    }
}

fn parse_until(s: &str) -> Result<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc));
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow!("{s:?} is not a time: use 2026-11-21 or 2026-11-21T10:00:00Z"))?;
    Ok(d.and_hms_opt(23, 59, 59).expect("valid time").and_utc())
}

fn human(b: u64) -> String {
    bookx_serve::devfs::human_size(b)
}

fn ts(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn run(cmd: BackupCmd, root: &Path, out: &mut dyn Write, err: &mut dyn Write) -> Result<i32> {
    let cancel = AtomicBool::new(false);
    match cmd {
        BackupCmd::Plan {
            place,
            encrypt,
            scope: sc,
            json,
        } => {
            let st = store(&place.to, &place.remote)?;
            st.check()?;
            let lib = LibrarySource::open(root)?;
            let repo = match Repository::open(st, &secrets()) {
                Ok(r) => Some(r),
                Err(e) if e.downcast_ref::<NoRepository>().is_some() => None,
                Err(e) => return Err(e),
            };
            let plan = match repo {
                Some(r) => r.plan(&lib, &scope(&sc), 3, &cancel, &|_| {})?,
                None => {
                    let free = store(&place.to, &place.remote)?.free_space();
                    estimate_new(&lib, &scope(&sc), encrypt, free)?
                }
            };
            if json {
                writeln!(
                    out,
                    "{}",
                    serde_json::json!({
                        "total_files": plan.total_files, "total_bytes": plan.total_bytes,
                        "new_files": plan.new_files, "new_bytes": plan.new_bytes,
                        "upload_bytes": plan.upload_bytes, "reused_files": plan.reused_files,
                        "catalogue_bytes": plan.catalogue_bytes, "free": plan.free,
                        "enough_space": plan.enough_space, "first_run": plan.first_run,
                        "estimated": plan.estimated, "too_big": plan.too_big,
                        "skipped": plan.skipped.iter().map(|s| serde_json::json!({"path": s.path, "reason": s.reason})).collect::<Vec<_>>(),
                    })
                )?;
            } else {
                writeln!(
                    out,
                    "{} file(s), {} in the library",
                    plan.total_files,
                    human(plan.total_bytes)
                )?;
                writeln!(
                    out,
                    "would copy {} new file(s), {} ({} stored), plus the catalogue ({}){}",
                    plan.new_files,
                    human(plan.new_bytes),
                    human(plan.upload_bytes),
                    human(plan.catalogue_bytes),
                    if plan.estimated {
                        " [estimate: the place is empty, so everything is new]"
                    } else {
                        ""
                    }
                )?;
                if plan.reused_files > 0 {
                    writeln!(out, "{} file(s) are already there", plan.reused_files)?;
                }
                match plan.free {
                    Some(f) => writeln!(
                        out,
                        "free space in the place: {}{}",
                        human(f),
                        if plan.enough_space {
                            ""
                        } else {
                            "  NOT ENOUGH"
                        }
                    )?,
                    None => writeln!(out, "free space in the place: unknown")?,
                }
                for t in &plan.too_big {
                    writeln!(out, "too big, left out: {t}")?;
                }
                for s in &plan.skipped {
                    writeln!(out, "left out: {} ({})", s.path, s.reason)?;
                }
            }
            Ok(if plan.enough_space { 0 } else { 1 })
        }
        BackupCmd::Run {
            place,
            encrypt,
            scope: sc,
            keep,
            manual,
            force,
            jobs,
            accept_rollback,
            gc_hours,
            progress,
            json,
        } => {
            let lib = LibrarySource::open(root)?;
            let repo = open_or_create(&place, encrypt)?;
            let opts = RunOptions {
                manual,
                scope: scope(&sc),
                keep,
                now: Utc::now(),
                jobs,
                force,
                accept_rollback,
                gc_grace: (gc_hours > 0).then(|| chrono::Duration::hours(gc_hours as i64)),
                ..RunOptions::default()
            };
            let cb = |p: &RunProgress| {
                if progress {
                    let _ = writeln!(
                        std::io::stderr().lock(),
                        "{}: {}/{} files, {}/{}{}",
                        p.phase.label(),
                        p.files_done,
                        p.files_total,
                        human(p.bytes_done),
                        human(p.bytes_total),
                        p.eta
                            .map(|e| format!(", about {} s left", e.as_secs()))
                            .unwrap_or_default()
                    );
                }
            };
            let r = repo.run(&lib, &opts, &cb, &cancel)?;
            if json {
                writeln!(
                    out,
                    "{}",
                    serde_json::json!({
                        "snapshot": r.snapshot, "seq": r.seq, "up_to_date": r.up_to_date,
                        "files": r.files, "total_bytes": r.total_bytes,
                        "new_files": r.new_files, "new_bytes": r.new_bytes, "stored_bytes": r.stored_bytes,
                        "reused_files": r.reused_files, "pruned_snapshots": r.pruned_snapshots,
                        "journal_records": r.journal_records, "verified": r.verified,
                        "too_big": r.too_big,
                        "skipped": r.skipped.iter().map(|s| serde_json::json!({"path": s.path, "reason": s.reason})).collect::<Vec<_>>(),
                    })
                )?;
            } else if r.up_to_date {
                writeln!(out, "up to date: nothing new to copy to {}", repo.label())?;
            } else {
                writeln!(
                    out,
                    "backed up {} file(s) to {} as {}: {} new ({}, {} stored), {} already there{}",
                    r.files,
                    repo.label(),
                    r.snapshot.as_deref().unwrap_or("?"),
                    r.new_files,
                    human(r.new_bytes),
                    human(r.stored_bytes),
                    r.reused_files,
                    if repo.is_encrypted() {
                        ", encrypted"
                    } else {
                        ""
                    }
                )?;
                if r.pruned_snapshots > 0 {
                    writeln!(out, "{} old snapshot(s) removed", r.pruned_snapshots)?;
                }
            }
            for s in &r.skipped {
                writeln!(err, "left out: {} ({})", s.path, s.reason)?;
            }
            for t in &r.too_big {
                writeln!(err, "too big, left out: {t}")?;
            }
            Ok(0)
        }
        BackupCmd::Journal { place, encrypt } => {
            let lib = LibrarySource::open(root)?;
            let repo = open_or_create(&place, encrypt)?;
            let j = repo.journal_append(&lib, Utc::now(), &cancel)?;
            match j.seq {
                Some(s) => writeln!(
                    out,
                    "journal record {s}: {} book(s) with changed notes",
                    j.books
                )?,
                None => writeln!(out, "no notes changed since the last record")?,
            }
            Ok(0)
        }
        BackupCmd::List {
            place,
            journal,
            json,
        } => {
            let repo = open_existing(&place)?;
            let snaps = repo.list_snapshots()?;
            let mut rows = Vec::new();
            for s in &snaps {
                let m = repo.manifest(&s.name);
                rows.push((s, m));
            }
            let jr = if journal {
                repo.list_journal()?
            } else {
                Vec::new()
            };
            if json {
                let v: Vec<_> = rows
                    .iter()
                    .map(|(s, m)| {
                        serde_json::json!({
                            "snapshot": s.file_name(), "seq": s.name.seq, "at": s.name.at.to_rfc3339(),
                            "kind": if s.name.manual { "manual" } else { "auto" },
                            "files": m.as_ref().ok().map(|m| m.files.len()),
                            "books": m.as_ref().ok().map(|m| m.books),
                            "bytes": m.as_ref().ok().map(|m| m.total_bytes()),
                            "error": m.as_ref().err().map(|e| format!("{e:#}")),
                        })
                    })
                    .collect();
                let j: Vec<_> = jr
                    .iter()
                    .map(|(seq, at, size)| serde_json::json!({"seq": seq, "at": at.to_rfc3339(), "bytes": size}))
                    .collect();
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"snapshots": v, "journal": j, "encrypted": repo.is_encrypted()})
                    )?
                )?;
            } else {
                if rows.is_empty() {
                    writeln!(out, "no snapshots in {}", repo.label())?;
                }
                for (s, m) in &rows {
                    match m {
                        Ok(m) => writeln!(
                            out,
                            "{}  #{:<4} {:<7} {:>6} books {:>7} files {:>10}  {}",
                            ts(s.name.at),
                            s.name.seq,
                            if s.name.manual { "manual" } else { "auto" },
                            m.books,
                            m.files.len(),
                            human(m.total_bytes()),
                            s.file_name()
                        )?,
                        Err(e) => writeln!(
                            out,
                            "{}  #{:<4} UNREADABLE: {e:#}",
                            ts(s.name.at),
                            s.name.seq
                        )?,
                    }
                }
                for (seq, at, size) in &jr {
                    writeln!(out, "{}  journal {seq} ({size} bytes)", ts(*at))?;
                }
            }
            Ok(0)
        }
        BackupCmd::Restore {
            snapshot,
            place,
            into,
            mode,
            dry_run,
            until,
            no_journal,
            no_files,
            no_catalogue,
            jobs,
            json,
        } => {
            let repo = open_existing(&place)?;
            let mode = match mode.as_str() {
                "missing-only" | "missing_only" => RestoreMode::MissingOnly,
                "replace-all" | "replace_all" => RestoreMode::ReplaceAll,
                other => bail!("unknown mode {other:?}: use missing-only or replace-all"),
            };
            let opts = RestoreOptions {
                mode,
                dry_run,
                until: until.as_deref().map(parse_until).transpose()?,
                restore_files: !no_files,
                restore_catalogue: !no_catalogue,
                replay_journal: !no_journal,
                jobs,
                ..RestoreOptions::default()
            };
            // a library that another Bookx has open is not restored over
            let _lock = if !dry_run && into.join(bookx_store::LIBRARY_DB).exists() {
                match bookx_store::guard::acquire(&into, "bookx-cli backup restore")? {
                    bookx_store::guard::Lock::Acquired(l) => Some(l),
                    bookx_store::guard::Lock::Busy(h) => {
                        bail!(
                            "{} is open in another Bookx ({}); close it first",
                            into.display(),
                            h.describe()
                        )
                    }
                }
            } else {
                None
            };
            let r = repo.restore(Some(&snapshot), &into, &opts, &|_| {}, &cancel)?;
            if json {
                writeln!(
                    out,
                    "{}",
                    serde_json::json!({
                        "snapshot": r.snapshot, "dry_run": r.dry_run, "files_total": r.files_total,
                        "files_written": r.files_written, "would_write": r.would_write, "bytes": r.bytes,
                        "skipped_existing": r.skipped_existing, "skipped_same": r.skipped_same,
                        "replaced_newer": r.replaced_newer,
                        "failed": r.failed.iter().map(|f| serde_json::json!({"path": f.0, "why": f.1})).collect::<Vec<_>>(),
                        "catalogue": format!("{:?}", r.catalogue),
                        "journal_records": r.journal_records, "journal_books_updated": r.journal_books_updated,
                    })
                )?;
            } else {
                writeln!(
                    out,
                    "{}{} from {}: {} of {} file(s) {}, {} already there, catalogue {:?}",
                    if r.dry_run {
                        "dry run, nothing changed. "
                    } else {
                        ""
                    },
                    r.snapshot,
                    ts(r.created.unwrap_or_default()),
                    if r.dry_run {
                        r.would_write
                    } else {
                        r.files_written
                    },
                    r.files_total,
                    if r.dry_run {
                        "would be written"
                    } else {
                        "written"
                    },
                    r.skipped_existing + r.skipped_same,
                    r.catalogue
                )?;
                if r.journal_records > 0 {
                    writeln!(
                        out,
                        "{} journal record(s) applied to {} book(s)",
                        r.journal_records, r.journal_books_updated
                    )?;
                }
                for p in &r.replaced_newer {
                    writeln!(out, "replaces a newer local file: {p}")?;
                }
            }
            for (p, why) in &r.failed {
                writeln!(err, "could not restore {p}: {why}")?;
            }
            for e in &r.journal_errors {
                writeln!(err, "journal: {e}")?;
            }
            Ok(if r.failed.is_empty() { 0 } else { 1 })
        }
        BackupCmd::Verify {
            place,
            deep,
            remove_bad,
            json,
        } => {
            let repo = open_existing(&place)?;
            let v = repo.verify(deep, 3, &cancel, &|_, _| {})?;
            if remove_bad {
                let ids: Vec<String> = v.corrupt.iter().map(|c| c.0.clone()).collect();
                let n = repo.remove_blobs(&ids)?;
                if n > 0 {
                    writeln!(
                        out,
                        "removed {n} damaged file(s); the next run copies them again"
                    )?;
                }
            }
            if json {
                writeln!(
                    out,
                    "{}",
                    serde_json::json!({
                        "ok": v.ok(), "deep": v.deep, "snapshots": v.snapshots_checked,
                        "blobs": v.blobs_checked, "bytes_read": v.bytes_read,
                        "missing": v.missing, "corrupt": v.corrupt,
                        "snapshots_bad": v.snapshots_bad, "journal_bad": v.journal_bad,
                    })
                )?;
            } else {
                writeln!(
                    out,
                    "{} snapshot(s), {} file(s) checked{}: {}",
                    v.snapshots_checked,
                    v.blobs_checked,
                    if deep {
                        format!(", {} read", human(v.bytes_read))
                    } else {
                        " (sizes only; --deep reads them)".into()
                    },
                    if v.ok() {
                        "all good".to_string()
                    } else {
                        "PROBLEMS FOUND".to_string()
                    }
                )?;
                for (n, why) in &v.snapshots_bad {
                    writeln!(out, "snapshot {n}: {why}")?;
                }
                for m in &v.missing {
                    writeln!(out, "missing: {m}")?;
                }
                for (id, why) in &v.corrupt {
                    writeln!(out, "damaged: {id}: {why}")?;
                }
                for (s, why) in &v.journal_bad {
                    writeln!(out, "journal {s}: {why}")?;
                }
            }
            Ok(if v.ok() { 0 } else { 1 })
        }
        BackupCmd::Gc {
            place,
            grace_hours,
            json,
        } => {
            let repo = open_existing(&place)?;
            let g = repo.gc(chrono::Duration::hours(grace_hours), Utc::now(), &cancel)?;
            if json {
                writeln!(
                    out,
                    "{}",
                    serde_json::json!({
                        "referenced": g.referenced, "in_place": g.blobs_in_place,
                        "newly_marked": g.newly_marked, "deleted_blobs": g.deleted_blobs,
                        "deleted_bytes": g.deleted_bytes, "temp_removed": g.temp_removed,
                    })
                )?;
            } else {
                writeln!(
                    out,
                    "{} file(s) in use; {} newly unused (removed after {} h), {} removed ({}), {} leftover temporary file(s) removed",
                    g.referenced,
                    g.newly_marked,
                    grace_hours,
                    g.deleted_blobs,
                    human(g.deleted_bytes),
                    g.temp_removed
                )?;
            }
            Ok(0)
        }
    }
}
