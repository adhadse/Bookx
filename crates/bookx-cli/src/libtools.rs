//! `bookx-cli library …` commands for library health and housekeeping: backups, restore, check,
//! Calibre import, layout, trash and undo, name tidying and the watched import folder.
//!
//! Commands that change the library take the library lock (`.bookx-lock`) and refuse with a clear
//! message when another Bookx has the library open.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Result};
use bookx_core::inbox::{self, InboxState};
use bookx_core::layout::Layout;
use bookx_core::library::{prepare_from_archive, Prepared};
use bookx_store::guard::{open_library_guarded, GuardedLibrary};
use bookx_store::health::{self, CheckOptions, Fix, Severity, SnapshotKind};
use clap::Subcommand;

/// How to reach a WebDAV destination.
#[derive(clap::Args, Debug, Default)]
pub struct RemoteArgs {
    /// WebDAV user name
    #[arg(long)]
    pub(crate) user: Option<String>,
    /// WebDAV: the folder below the address
    #[arg(long, default_value = "Bookx backups")]
    pub(crate) folder: String,
}

fn destination(spec: &str, r: &RemoteArgs) -> Result<Box<dyn bookx_backup::Target>> {
    if spec.starts_with("http://") || spec.starts_with("https://") {
        let pw = std::env::var("BOOKX_WEBDAV_PASSWORD").unwrap_or_default();
        return Ok(Box::new(bookx_backup::WebDavTarget::new(
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
    Ok(Box::new(bookx_backup::FolderTarget { dir: spec.into() }))
}

fn passphrase() -> Option<String> {
    std::env::var("BOOKX_BACKUP_PASSPHRASE")
        .ok()
        .filter(|p| !p.is_empty())
}

#[derive(Subcommand, Debug)]
pub enum ToolsCmd {
    /// Write a snapshot of the catalogue into <library>/.bookx-backups, and with --to also copy
    /// it to a folder (external drive, mounted share, cloud-drive mount) or a WebDAV address
    Backup {
        /// A folder, or an https:// WebDAV address (password in BOOKX_WEBDAV_PASSWORD)
        #[arg(long)]
        to: Option<String>,
        #[command(flatten)]
        remote: RemoteArgs,
        /// Encrypt the copy (passphrase in BOOKX_BACKUP_PASSPHRASE)
        #[arg(long, requires = "to")]
        encrypt: bool,
        /// How many automatic backups the destination keeps (manual ones are never removed)
        #[arg(long, default_value_t = 10)]
        keep: usize,
    },
    /// List the snapshots, here or at a destination (--from)
    Snapshots {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        from: Option<String>,
        #[command(flatten)]
        remote: RemoteArgs,
    },
    /// Restore the catalogue from a snapshot (a file path, a file name from `snapshots`, or
    /// `latest`); with --from the name is looked up at that destination. Encrypted backups
    /// (.bookxbackup) need BOOKX_BACKUP_PASSPHRASE.
    Restore {
        snapshot: String,
        #[arg(long)]
        from: Option<String>,
        #[command(flatten)]
        remote: RemoteArgs,
    },
    /// Check the database, the files, the covers and (with --drift) file against catalogue
    Check {
        /// Also compare every book's metadata with the file's
        #[arg(long)]
        drift: bool,
        /// Apply the first suggested fix to each problem (missing files are only removed from the
        /// catalogue, into the trash)
        #[arg(long)]
        fix: bool,
        #[arg(long)]
        json: bool,
    },
    /// Import a Calibre library (its metadata.db) with progress and a report
    ImportCalibre {
        /// The Calibre library folder
        dir: PathBuf,
        /// Copy the book files into this library (default: keep them in the Calibre folder)
        #[arg(long)]
        copy: bool,
    },
    /// Write the library as a Calibre library (Author/Title (id)/ files, cover.jpg, metadata.opf
    /// and a metadata.db Calibre can open) into an empty folder. The library is only read.
    ExportCalibre {
        /// The folder to create (it must not exist or be empty, and not be inside the library)
        dir: PathBuf,
    },
    /// Show or change where books are stored: tree, flat, template, linked
    Layout {
        /// tree | flat | template | linked (omit to show the current one)
        name: Option<String>,
        /// The template for `template`, for example "{author}/{series}/{title}"
        #[arg(long, default_value = "")]
        template: String,
        /// Only list what would move
        #[arg(long)]
        dry_run: bool,
    },
    /// The trash of deleted books
    #[command(subcommand)]
    Trash(TrashCmd),
    /// The log of library operations
    History {
        #[arg(long)]
        json: bool,
    },
    /// Take back the last operation (import, delete, edit, move)
    Undo,
    /// Tidy author and series names across the library
    Tidy {
        /// Write the changes (default: only list them)
        #[arg(long)]
        apply: bool,
    },
    /// The watched import folder and its review queue
    #[command(subcommand)]
    Inbox(InboxCmd),
}

#[derive(Subcommand, Debug)]
pub enum TrashCmd {
    List,
    /// Put a book back (the id shown by `trash list`)
    Restore {
        id: u64,
    },
    /// Erase everything in the trash for good
    Empty,
}

#[derive(Subcommand, Debug)]
pub enum InboxCmd {
    /// Set the watched folder
    Set { folder: PathBuf },
    /// List the new books waiting in the folder
    Scan {
        /// Seconds a file must be unchanged before it is listed
        #[arg(long, default_value_t = 3)]
        settle: u64,
    },
    /// Add the waiting books (duplicates are left out)
    AddAll {
        #[arg(long, default_value_t = 3)]
        settle: u64,
    },
    /// Forget the waiting books without adding them
    DismissAll {
        #[arg(long, default_value_t = 3)]
        settle: u64,
    },
}

/// Opens the library for changes, or fails with the message about who else has it.
pub fn open_for_write(root: &Path) -> Result<GuardedLibrary> {
    let g = open_library_guarded(root, "Bookx CLI", false)?;
    if let Some(h) = &g.busy {
        bail!("{}", h.describe());
    }
    Ok(g)
}

/// Where the fields of a book came from, as one line (empty when nothing was recorded).
pub fn sources_line(root: &Path, id: u64) -> String {
    let Ok(g) = open_library_guarded(root, "Bookx CLI", true) else {
        return String::new();
    };
    let mut by_source: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for f in g.opened.store.provenance(id) {
        by_source.entry(f.source).or_default().push(f.field);
    }
    by_source
        .into_iter()
        .map(|(s, f)| format!("{s}: {}", f.join(", ")))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Reads any supported file into something the library can import.
fn prepare_any(path: &Path) -> Result<Prepared> {
    if bookx_core::library::is_supported_file(path) {
        return bookx_core::library::prepare_import(path);
    }
    let bytes = std::fs::read(path)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("input");
    let archive = crate::read_archive(name, &bytes)?;
    prepare_from_archive(archive, path)
}

fn existing(root: &Path) -> Result<()> {
    if !bookx_store::has_library(root) {
        bail!(
            "no library at {} (add books first, or pass --library)",
            root.display()
        );
    }
    Ok(())
}

pub fn run(cmd: ToolsCmd, root: &Path, out: &mut dyn Write, err: &mut dyn Write) -> Result<i32> {
    existing_or_import(&cmd, root)?;
    match cmd {
        ToolsCmd::Backup {
            to,
            remote,
            encrypt,
            keep,
        } => {
            let g = open_for_write(root)?;
            let s = health::take_snapshot(
                &g.opened.store,
                root,
                SnapshotKind::Manual,
                chrono::Utc::now(),
            )?;
            writeln!(out, "backed up {} book(s) to {}", s.books, s.path.display())?;
            if let Some(to) = to {
                let pass = if encrypt {
                    Some(passphrase().ok_or_else(|| {
                        anyhow!("--encrypt needs the passphrase in BOOKX_BACKUP_PASSPHRASE")
                    })?)
                } else {
                    None
                };
                let target = destination(&to, &remote)?;
                let opts = bookx_backup::PushOptions {
                    passphrase: pass.as_deref(),
                    keep,
                    now: chrono::Utc::now(),
                    kdf: bookx_backup::crypto::Kdf::DEFAULT,
                };
                let r = bookx_backup::push(
                    target.as_ref(),
                    &s,
                    &opts,
                    &std::sync::atomic::AtomicBool::new(false),
                    &|_| {},
                )?;
                writeln!(
                    out,
                    "copied to {} as {} ({} bytes{}, checked after writing{})",
                    target.label(),
                    r.name,
                    r.bytes,
                    if r.encrypted { ", encrypted" } else { "" },
                    if r.pruned > 0 {
                        format!(", {} old backup(s) removed", r.pruned)
                    } else {
                        String::new()
                    }
                )?;
            }
            Ok(0)
        }
        ToolsCmd::Snapshots {
            from: Some(from),
            remote,
            json,
        } => {
            let target = destination(&from, &remote)?;
            let list = bookx_backup::list_backups(target.as_ref())?;
            if json {
                let v: Vec<_> = list
                    .iter()
                    .map(|(f, n)| {
                        serde_json::json!({
                            "file": f.name, "at": n.at.to_rfc3339(), "books": n.books,
                            "kind": n.kind.label(), "bytes": f.size, "encrypted": n.encrypted,
                        })
                    })
                    .collect();
                writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
            } else if list.is_empty() {
                writeln!(out, "no backups at {}", target.label())?;
            } else {
                for (f, n) in &list {
                    writeln!(
                        out,
                        "{}  {:>6} books  {:<17} {}{}",
                        n.at.format("%Y-%m-%d %H:%M:%S"),
                        n.books,
                        n.kind.label(),
                        f.name,
                        if n.encrypted { "  (encrypted)" } else { "" }
                    )?;
                }
            }
            Ok(0)
        }
        ToolsCmd::Snapshots { json, .. } => {
            let snaps = health::list_snapshots(root);
            if json {
                let v: Vec<_> = snaps
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "file": s.path.file_name().map(|n| n.to_string_lossy().to_string()),
                            "path": s.path,
                            "at": s.at.to_rfc3339(),
                            "books": s.books,
                            "kind": s.kind.label(),
                            "bytes": s.size,
                        })
                    })
                    .collect();
                writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
            } else if snaps.is_empty() {
                writeln!(out, "no snapshots yet (run `library backup`)")?;
            } else {
                for s in &snaps {
                    writeln!(
                        out,
                        "{}  {:>6} books  {:<17} {}",
                        s.at.format("%Y-%m-%d %H:%M:%S"),
                        s.books,
                        s.kind.label(),
                        s.path.file_name().unwrap_or_default().to_string_lossy()
                    )?;
                }
            }
            Ok(0)
        }
        ToolsCmd::Restore {
            snapshot,
            from,
            remote,
        } => {
            let tmp = root.join(health::BACKUP_DIR).join("restore-tmp");
            let path = match from {
                Some(from) => {
                    let target = destination(&from, &remote)?;
                    let name = if snapshot == "latest" {
                        bookx_backup::list_backups(target.as_ref())?
                            .into_iter()
                            .next()
                            .map(|(f, _)| f.name)
                            .ok_or_else(|| anyhow!("there are no backups at {}", target.label()))?
                    } else {
                        snapshot.clone()
                    };
                    bookx_backup::fetch_to_file(
                        target.as_ref(),
                        &name,
                        passphrase().as_deref(),
                        &tmp,
                    )?
                }
                None => {
                    let p = resolve_snapshot(root, &snapshot)?;
                    // a .bookxbackup picked by path is decrypted first
                    if std::fs::read(&p).is_ok_and(|b| bookx_backup::crypto::is_encrypted(&b)) {
                        bookx_backup::open_file(&p, passphrase().as_deref(), &tmp)?
                    } else {
                        p
                    }
                }
            };
            let g = open_for_write(root)?;
            let r = health::restore_snapshot(&g.opened.store, root, &path, chrono::Utc::now());
            let _ = std::fs::remove_dir_all(&tmp);
            r?;
            writeln!(
                out,
                "restored {} book(s) from {} (the previous catalogue was kept as a \"Before a restore\" snapshot)",
                g.opened.store.len(),
                path.display()
            )?;
            Ok(0)
        }
        ToolsCmd::Check { drift, fix, json } => check(root, drift, fix, json, out),
        ToolsCmd::ImportCalibre { dir, copy } => {
            let g = open_for_write(root)?;
            let mut last = (None, 0usize);
            let r = bookx_store::import_calibre_into(
                &g.opened.store,
                root,
                &dir,
                copy,
                &mut |stage, done, total| {
                    let pct = if total == 0 { 100 } else { done * 100 / total };
                    if last.0 != Some(stage) || last.1 != pct {
                        let what = match stage {
                            bookx_store::Stage::Reading => "reading",
                            bookx_store::Stage::Copying => "copying",
                        };
                        let _ = writeln!(err, "{what}: {done}/{total}");
                        last = (Some(stage), pct);
                    }
                },
            )?;
            writeln!(out, "{}", r.summary())?;
            Ok(if r.copy_failed.is_empty() { 0 } else { 1 })
        }
        ToolsCmd::ExportCalibre { dir } => {
            let g = open_library_guarded(root, "Bookx CLI", true)?;
            let mut last = usize::MAX;
            let r = bookx_store::export_calibre(
                &g.opened.library.books,
                root,
                &dir,
                &mut |done, total| {
                    let pct = if total == 0 { 100 } else { done * 100 / total };
                    if pct != last {
                        last = pct;
                        let _ = writeln!(err, "exporting: {done}/{total}");
                    }
                },
                &|| false,
            )?;
            writeln!(out, "{} to {}", r.summary(), dir.display())?;
            Ok(if r.problems.is_empty() { 0 } else { 1 })
        }
        ToolsCmd::Layout {
            name,
            template,
            dry_run,
        } => layout(root, name, &template, dry_run, out),
        ToolsCmd::Trash(t) => trash(t, root, out),
        ToolsCmd::History { json } => {
            let g = open_library_guarded(root, "Bookx CLI", true)?;
            let ops = g.opened.library.history();
            if json {
                writeln!(out, "{}", serde_json::to_string_pretty(ops)?)?;
            } else if ops.is_empty() {
                writeln!(out, "no operations logged yet")?;
            } else {
                for o in ops {
                    writeln!(
                        out,
                        "#{:<4} {}  {}{}",
                        o.seq,
                        o.at.format("%Y-%m-%d %H:%M:%S"),
                        o.label,
                        if o.undone { "  (undone)" } else { "" }
                    )?;
                }
            }
            Ok(0)
        }
        ToolsCmd::Undo => {
            let mut g = open_for_write(root)?;
            match g.opened.library.undo_last()? {
                Some(label) => {
                    g.opened.library.save()?;
                    writeln!(out, "undone: {label}")?;
                    Ok(0)
                }
                None => {
                    writeln!(out, "nothing to undo")?;
                    Ok(1)
                }
            }
        }
        ToolsCmd::Tidy { apply } => {
            let mut g = if apply {
                open_for_write(root)?
            } else {
                open_library_guarded(root, "Bookx CLI", true)?
            };
            let changes = bookx_sources::tidy::plan(&g.opened.library.books);
            for c in &changes {
                let f = match c.field {
                    bookx_sources::tidy::Field::Authors => "author",
                    bookx_sources::tidy::Field::Series => "series",
                };
                writeln!(
                    out,
                    "#{} {}: “{}” → “{}”  ({})",
                    c.id, f, c.from, c.to, c.title
                )?;
            }
            if apply {
                let n = bookx_sources::tidy::apply(&mut g.opened.library, &changes)?;
                g.opened.library.save()?;
                writeln!(out, "{n} book(s) changed")?;
            } else {
                writeln!(
                    out,
                    "{} change(s) proposed (use --apply to write them)",
                    changes.len()
                )?;
            }
            Ok(0)
        }
        ToolsCmd::Inbox(c) => inbox_cmd(c, root, out),
    }
}

/// Everything except a Calibre import needs a library that exists.
fn existing_or_import(cmd: &ToolsCmd, root: &Path) -> Result<()> {
    match cmd {
        ToolsCmd::ImportCalibre { .. } | ToolsCmd::Layout { .. } => Ok(()),
        _ => existing(root),
    }
}

fn resolve_snapshot(root: &Path, name: &str) -> Result<PathBuf> {
    let snaps = health::list_snapshots(root);
    if name == "latest" {
        return snaps
            .first()
            .map(|s| s.path.clone())
            .ok_or_else(|| anyhow!("there are no snapshots"));
    }
    let direct = PathBuf::from(name);
    if direct.is_file() {
        return Ok(direct);
    }
    snaps
        .iter()
        .find(|s| s.path.file_name().is_some_and(|n| n == name))
        .map(|s| s.path.clone())
        .ok_or_else(|| anyhow!("no snapshot {name} (see `library snapshots`)"))
}

fn check(root: &Path, drift: bool, fix: bool, json: bool, out: &mut dyn Write) -> Result<i32> {
    let mut g = if fix {
        open_for_write(root)?
    } else {
        open_library_guarded(root, "Bookx CLI", true)?
    };
    let report = health::check_library(
        &g.opened.library,
        Some(&g.opened.store),
        CheckOptions { drift },
    );
    if json {
        let v: Vec<_> = report
            .findings
            .iter()
            .map(|f| {
                serde_json::json!({
                    "severity": match f.severity { Severity::Error => "error", Severity::Warning => "warning", Severity::Info => "info" },
                    "book": f.book,
                    "message": f.message,
                })
            })
            .collect();
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "books": report.books_checked,
                "findings": v,
            }))?
        )?;
    } else {
        for f in &report.findings {
            let tag = match f.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "note",
            };
            writeln!(out, "{tag}: {}", f.message)?;
        }
        writeln!(
            out,
            "{} book(s) checked, {} problem(s)",
            report.books_checked,
            report.findings.len()
        )?;
    }
    if fix {
        for f in &report.findings {
            if let Some(fx) = f.fixes.first() {
                let fx: &Fix = fx;
                match health::apply_fix(&mut g.opened.library, f, fx) {
                    Ok(m) => writeln!(out, "fixed: {m}")?,
                    Err(e) => writeln!(out, "could not fix: {e:#}")?,
                }
            }
        }
        g.opened.library.save()?;
    }
    Ok(if report.errors() > 0 { 1 } else { 0 })
}

fn layout(
    root: &Path,
    name: Option<String>,
    template: &str,
    dry_run: bool,
    out: &mut dyn Write,
) -> Result<i32> {
    let name = match name {
        None => {
            existing(root)?;
            let g = open_library_guarded(root, "Bookx CLI", true)?;
            let l = g.opened.library.layout();
            match l {
                Layout::Template(t) => writeln!(out, "template: {t}")?,
                Layout::Tree => writeln!(out, "tree")?,
                Layout::Flat => writeln!(out, "flat")?,
                Layout::Linked => writeln!(out, "linked")?,
            }
            return Ok(0);
        }
        Some(n) => n,
    };
    let new = Layout::parse(&name, template)
        .ok_or_else(|| anyhow!("unknown layout {name} (tree, flat, template, linked)"))?;
    existing(root)?;
    let mut g = if dry_run {
        open_library_guarded(root, "Bookx CLI", true)?
    } else {
        open_for_write(root)?
    };
    if let Layout::Template(t) = &new {
        let f: bookx_core::layout::TemplateFn = bookx_sources::template::render_library_path;
        let sample =
            bookx_core::layout::check_template(t, f).map_err(|e| anyhow!("bad template: {e}"))?;
        writeln!(out, "template renders e.g. {sample}")?;
    }
    let plan = g.opened.library.plan_layout(&new);
    for p in &plan {
        writeln!(out, "{} → {}", p.old_file, p.new_file)?;
    }
    if dry_run {
        writeln!(
            out,
            "{} book(s) would move; nothing was changed",
            plan.len()
        )?;
        return Ok(0);
    }
    let n = g.opened.library.change_layout(new)?;
    writeln!(out, "{n} book(s) moved; the layout is now {name}")?;
    Ok(0)
}

fn trash(cmd: TrashCmd, root: &Path, out: &mut dyn Write) -> Result<i32> {
    match cmd {
        TrashCmd::List => {
            let g = open_library_guarded(root, "Bookx CLI", true)?;
            let t = g.opened.library.trash();
            if t.is_empty() {
                writeln!(out, "the trash is empty")?;
            }
            for e in t {
                writeln!(
                    out,
                    "#{:<5} {}  {} — {}",
                    e.book.id,
                    e.at.format("%Y-%m-%d %H:%M"),
                    e.book.meta.title,
                    e.book.meta.authors_display()
                )?;
            }
            Ok(0)
        }
        TrashCmd::Restore { id } => {
            let mut g = open_for_write(root)?;
            let name = g
                .opened
                .library
                .trash()
                .into_iter()
                .find(|e| e.book.id == id)
                .map(|e| e.name)
                .ok_or_else(|| anyhow!("book #{id} is not in the trash"))?;
            g.opened.library.restore_from_trash(&name)?;
            writeln!(out, "restored #{id}")?;
            Ok(0)
        }
        TrashCmd::Empty => {
            let mut g = open_for_write(root)?;
            let n = g.opened.library.empty_trash()?;
            writeln!(out, "erased {n} book(s) for good")?;
            Ok(0)
        }
    }
}

fn inbox_cmd(cmd: InboxCmd, root: &Path, out: &mut dyn Write) -> Result<i32> {
    let mut state = InboxState::load(root);
    match cmd {
        InboxCmd::Set { folder } => {
            if !folder.is_dir() {
                bail!("{} is not a folder", folder.display());
            }
            state.folder = Some(folder.clone());
            state.save(root)?;
            writeln!(out, "watching {}", folder.display())?;
            Ok(0)
        }
        InboxCmd::Scan { settle } => {
            let g = open_library_guarded(root, "Bookx CLI", true)?;
            let found = inbox::scan(
                &state,
                &g.opened.library,
                SystemTime::now(),
                std::time::Duration::from_secs(settle),
                &prepare_any,
            );
            for c in &found {
                let note = match (&c.error, c.duplicate_of) {
                    (Some(e), _) => format!("  [cannot read: {e}]"),
                    (None, Some(id)) => format!("  [looks like #{id}]"),
                    _ => String::new(),
                };
                writeln!(out, "{} — {}{note}", c.title, c.path.display())?;
            }
            writeln!(out, "{} book(s) waiting", found.len())?;
            Ok(0)
        }
        InboxCmd::AddAll { settle } => {
            let mut g = open_for_write(root)?;
            let found = inbox::scan(
                &state,
                &g.opened.library,
                SystemTime::now(),
                std::time::Duration::from_secs(settle),
                &prepare_any,
            );
            let (mut added, mut dups, mut failed) = (0, 0, 0);
            for c in &found {
                if c.error.is_some() {
                    failed += 1;
                    continue;
                }
                match inbox::accept(&mut state, &mut g.opened.library, c, false, &prepare_any) {
                    Ok(bookx_core::library::AddOutcome::Added(_)) => added += 1,
                    Ok(bookx_core::library::AddOutcome::Duplicate(_)) => dups += 1,
                    Err(_) => failed += 1,
                }
            }
            state.save(root)?;
            writeln!(
                out,
                "{added} added, {dups} duplicate(s) left out, {failed} failed"
            )?;
            Ok(if failed > 0 { 1 } else { 0 })
        }
        InboxCmd::DismissAll { settle } => {
            let g = open_library_guarded(root, "Bookx CLI", true)?;
            let found = inbox::scan(
                &state,
                &g.opened.library,
                SystemTime::now(),
                std::time::Duration::from_secs(settle),
                &prepare_any,
            );
            for c in &found {
                inbox::dismiss(&mut state, c);
            }
            state.save(root)?;
            writeln!(out, "{} book(s) dismissed", found.len())?;
            Ok(0)
        }
    }
}

/// Registers the template renderer for layouts; called once by [`crate::run`].
pub fn init() {
    bookx_core::layout::register_template_renderer(bookx_sources::template::render_library_path);
}
