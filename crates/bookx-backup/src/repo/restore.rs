//! Getting a library back: files, catalogue, and the journal on top, with a dry run and with every
//! path from the repository treated as hostile.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use bookx_store::health::{self, SnapshotKind};
use bookx_store::{LIBRARY_DB, ReadingStatus, SqliteStore};
use chrono::{DateTime, Utc};

use super::manifest::{FileEntry, MAX_FILES, MAX_TOTAL_BYTES, Manifest, validate_rel};
use super::run::{Phase, Reporter, RunProgress, parallel_for};
use super::seal::ids_equal;
use super::source::TMP_DIR;
use super::{Repository, RolledBack};
use crate::push::Cancelled;

/// What to do with files that already exist in the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RestoreMode {
    /// Only write what is not there; an existing file or catalogue is never touched.
    #[default]
    MissingOnly,
    /// Make the target match the snapshot: files that differ are replaced (and listed in
    /// `replaced_newer` when the local one was newer), the catalogue is replaced after a
    /// "before a restore" snapshot of the old one.
    ReplaceAll,
}

#[derive(Debug, Clone)]
pub struct RestoreOptions {
    pub mode: RestoreMode,
    /// Report what would be done and change nothing.
    pub dry_run: bool,
    /// Use the newest snapshot at or before this time, and replay the journal up to it.
    pub until: Option<DateTime<Utc>>,
    pub restore_files: bool,
    pub restore_catalogue: bool,
    /// After the catalogue, apply journal records newer than the snapshot.
    pub replay_journal: bool,
    /// Refuse a snapshot older than this number (the newest one this library has seen).
    pub min_seq: Option<u64>,
    pub jobs: usize,
    pub max_files: usize,
    pub max_bytes: u64,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        RestoreOptions {
            mode: RestoreMode::MissingOnly,
            dry_run: false,
            until: None,
            restore_files: true,
            restore_catalogue: true,
            replay_journal: true,
            min_seq: None,
            jobs: 3,
            max_files: MAX_FILES,
            max_bytes: MAX_TOTAL_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CatalogueAction {
    #[default]
    NotRequested,
    /// Dry run: it would be written.
    WouldRestore,
    Restored,
    /// An existing catalogue was kept (`MissingOnly`).
    KeptExisting,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    pub snapshot: String,
    pub seq: u64,
    pub created: Option<DateTime<Utc>>,
    pub files_total: usize,
    pub files_written: usize,
    /// Files that need writing (dry run), or were written.
    pub would_write: usize,
    pub bytes: u64,
    pub skipped_existing: usize,
    pub skipped_same: usize,
    /// Files replaced although the local one had a newer modification time.
    pub replaced_newer: Vec<String>,
    /// Files that could not be restored: (path, why). The rest was restored.
    pub failed: Vec<(String, String)>,
    pub catalogue: CatalogueAction,
    pub journal_records: usize,
    pub journal_books_updated: usize,
    pub journal_books_missing: usize,
    pub journal_collections: usize,
    pub journal_errors: Vec<String>,
    pub dry_run: bool,
}

/// What [`Repository::journal_replay`] did.
#[derive(Debug, Clone, Default)]
pub struct ReplayReport {
    pub records: usize,
    pub books_updated: usize,
    /// Notes of books the restored catalogue does not have (they are not recreated).
    pub books_missing: usize,
    /// Collections whose books were set from the journal.
    pub collections_updated: usize,
    pub errors: Vec<String>,
}

enum Action {
    Write { newer_local: bool },
    SkipExisting,
    SkipSame,
    Blocked(String),
}

/// The destination of a manifest path below `root`, with its folders created, every existing
/// component checked not to be a link. Never leaves `root`.
fn prepare_dest(root: &Path, rel: &str) -> Result<PathBuf> {
    validate_rel(rel)?;
    let parts: Vec<&str> = rel.split('/').collect();
    let mut cur = root.to_path_buf();
    for c in &parts[..parts.len() - 1] {
        cur.push(c);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                bail!("{} is a link; not following it", cur.display())
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => bail!("a file is in the way of {}", cur.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match std::fs::create_dir(&cur) {
                Ok(()) => {}
                // another worker made it a moment ago: fine if it is a real folder
                Err(_) if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.is_dir()) => {}
                Err(e) => bail!("cannot create {}: {e}", cur.display()),
            },
            Err(e) => bail!("cannot look at {}: {e}", cur.display()),
        }
    }
    Ok(cur.join(parts[parts.len() - 1]))
}

fn dest_state(root: &Path, rel: &str) -> Option<std::fs::Metadata> {
    std::fs::symlink_metadata(root.join(rel)).ok()
}

impl Repository {
    /// Restores a snapshot (`None` or `"latest"`: the newest) into `target`, a new or existing
    /// library folder. See [`RestoreOptions`] for the modes; with `dry_run` nothing is changed.
    /// Every restored file is checked against its hash; a file that fails is reported in
    /// `failed` and the others are still restored.
    pub fn restore(
        &self,
        snapshot: Option<&str>,
        target: &Path,
        opts: &RestoreOptions,
        progress: &(dyn Fn(&RunProgress) + Sync),
        cancel: &AtomicBool,
    ) -> Result<RestoreReport> {
        let info = self.find_snapshot(snapshot, opts.until)?;
        let m = self.manifest(&info.name)?;
        if let Some(min) = opts.min_seq {
            if m.seq < min && (snapshot.is_none() || snapshot == Some("latest")) {
                return Err(anyhow::Error::new(RolledBack {
                    seen: min,
                    found: m.seq,
                }));
            }
        }
        if m.files.len() > opts.max_files || m.total_bytes() > opts.max_bytes {
            bail!(
                "the snapshot has {} files and {} bytes: more than the restore limits",
                m.files.len(),
                m.total_bytes()
            );
        }
        let mut rep = RestoreReport {
            snapshot: info.name.file_name(),
            seq: m.seq,
            created: Some(m.created),
            files_total: m.files.len(),
            dry_run: opts.dry_run,
            ..RestoreReport::default()
        };
        if !opts.dry_run {
            std::fs::create_dir_all(target)
                .with_context(|| format!("cannot create {}", target.display()))?;
        }
        let root = if target.exists() {
            std::fs::canonicalize(target)?
        } else {
            target.to_path_buf()
        };
        let rp = Reporter::new(progress);

        if opts.restore_files {
            self.restore_files(&m, &root, opts, &rp, cancel, &mut rep)?;
        }
        if opts.restore_catalogue {
            self.restore_catalogue(&m, &root, opts, cancel, &mut rep)?;
        }
        rp.phase(Phase::Done, 0, 0);
        Ok(rep)
    }

    fn restore_files(
        &self,
        m: &Manifest,
        root: &Path,
        opts: &RestoreOptions,
        rp: &Reporter,
        cancel: &AtomicBool,
        rep: &mut RestoreReport,
    ) -> Result<()> {
        let files = &m.files;
        // 1. decide, file by file (hashing existing files in ReplaceAll mode)
        rp.phase(Phase::Hashing, files.len() as u64, m.total_bytes());
        let actions: Mutex<Vec<Option<Action>>> =
            Mutex::new((0..files.len()).map(|_| None).collect());
        parallel_for(files.len(), opts.jobs, |i| {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let e = &files[i];
            rp.current(&e.path);
            let a = self.classify(root, e, opts);
            actions.lock().unwrap()[i] = Some(a);
            rp.files_done.fetch_add(1, Ordering::Relaxed);
            rp.bytes_done.fetch_add(e.size, Ordering::Relaxed);
            rp.emit(false);
        });
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow::Error::new(Cancelled));
        }
        let actions: Vec<Action> = actions
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|a| a.expect("every file was classified"))
            .collect();
        let mut todo: Vec<usize> = Vec::new();
        for (i, a) in actions.iter().enumerate() {
            match a {
                Action::Write { newer_local } => {
                    todo.push(i);
                    rep.bytes += files[i].size;
                    if *newer_local {
                        rep.replaced_newer.push(files[i].path.clone());
                    }
                }
                Action::SkipExisting => rep.skipped_existing += 1,
                Action::SkipSame => rep.skipped_same += 1,
                Action::Blocked(why) => rep.failed.push((files[i].path.clone(), why.clone())),
            }
        }
        rep.would_write = todo.len();
        if opts.dry_run {
            return Ok(());
        }
        // 2. room
        if let Some(sp) = bookx_serve::devfs::free_space(root) {
            if sp.free < rep.bytes {
                bail!(
                    "not enough space in {}: {} needed, {} free",
                    root.display(),
                    super::run::human(rep.bytes),
                    super::run::human(sp.free)
                );
            }
        }
        // 3. write
        rp.phase(Phase::Uploading, todo.len() as u64, rep.bytes);
        let failed: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
        let written = std::sync::atomic::AtomicUsize::new(0);
        parallel_for(todo.len(), opts.jobs, |k| {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let e = &files[todo[k]];
            rp.current(&e.path);
            match self.restore_one(root, e, cancel) {
                Ok(()) => {
                    written.fetch_add(1, Ordering::Relaxed);
                }
                Err(err) => {
                    if !cancel.load(Ordering::Relaxed) {
                        failed
                            .lock()
                            .unwrap()
                            .push((e.path.clone(), format!("{err:#}")));
                    }
                }
            }
            rp.files_done.fetch_add(1, Ordering::Relaxed);
            rp.bytes_done.fetch_add(e.size, Ordering::Relaxed);
            rp.emit(true);
        });
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow::Error::new(Cancelled));
        }
        rep.files_written = written.load(Ordering::Relaxed);
        let mut f = failed.into_inner().unwrap();
        f.sort();
        rep.failed.extend(f);
        Ok(())
    }

    fn classify(&self, root: &Path, e: &FileEntry, opts: &RestoreOptions) -> Action {
        if let Err(why) = validate_rel(&e.path) {
            return Action::Blocked(format!("{why}"));
        }
        let Some(meta) = dest_state(root, &e.path) else {
            return Action::Write { newer_local: false };
        };
        if opts.mode == RestoreMode::MissingOnly {
            return Action::SkipExisting;
        }
        if meta.is_dir() {
            return Action::Blocked("a folder is in the way".into());
        }
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Action::Write { newer_local: false };
        }
        if meta.len() == e.size {
            if let Ok(mut f) = std::fs::File::open(root.join(&e.path)) {
                let mut h = self.crypto.id_hasher();
                let mut buf = vec![0u8; 256 * 1024];
                let mut ok = true;
                loop {
                    match f.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => h.update(&buf[..n]),
                        Err(_) => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok && ids_equal(&h.finish_hex(), &e.blob) {
                    return Action::SkipSame;
                }
            }
        }
        let local = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Action::Write {
            newer_local: local > e.mtime,
        }
    }

    fn restore_one(&self, root: &Path, e: &FileEntry, cancel: &AtomicBool) -> Result<()> {
        let dest = prepare_dest(root, &e.path)?;
        let parent = dest.parent().expect("has a parent");
        let tmp = parent.join(format!(".bookx-restore-{}", super::seal::random_hex(6)?));
        let res = (|| -> Result<()> {
            let mut f = std::fs::File::create_new(&tmp)
                .with_context(|| format!("cannot write in {}", parent.display()))?;
            {
                let mut w = CancelWrite {
                    inner: &mut f,
                    cancel,
                };
                self.read_blob(&e.blob, e.size, &mut w)?;
            }
            if e.mtime > 0 {
                let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(e.mtime as u64);
                let _ = f.set_modified(t);
            }
            drop(f);
            // replaces a file or a link at the destination, never follows it
            if let Ok(m) = std::fs::symlink_metadata(&dest) {
                if m.is_dir() {
                    bail!("a folder is in the way");
                }
            }
            std::fs::rename(&tmp, &dest).with_context(|| format!("cannot write {}", dest.display()))
        })();
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }

    fn restore_catalogue(
        &self,
        m: &Manifest,
        root: &Path,
        opts: &RestoreOptions,
        cancel: &AtomicBool,
        rep: &mut RestoreReport,
    ) -> Result<()> {
        let db = root.join(LIBRARY_DB);
        let had = db.exists();
        if had && opts.mode == RestoreMode::MissingOnly {
            rep.catalogue = CatalogueAction::KeptExisting;
            return Ok(());
        }
        if opts.dry_run {
            rep.catalogue = CatalogueAction::WouldRestore;
            return Ok(());
        }
        let tmpdir = root.join(TMP_DIR);
        std::fs::create_dir_all(&tmpdir)?;
        let tmp = tmpdir.join(format!("restore-{}.db", super::seal::random_hex(6)?));
        let res = (|| -> Result<()> {
            {
                let mut f = std::fs::File::create_new(&tmp)?;
                let mut w = CancelWrite {
                    inner: &mut f,
                    cancel,
                };
                self.read_catalogue(m, &mut w)?;
            }
            let problems = SqliteStore::check_file(&tmp);
            if let Some(p) = problems.first() {
                bail!("the catalogue in the snapshot is damaged: {p}");
            }
            let store = SqliteStore::open(&db)?;
            if had {
                health::take_snapshot(&store, root, SnapshotKind::PreRestore, Utc::now())?;
            }
            store.restore_from(&tmp)?;
            if opts.replay_journal {
                let r = self.journal_replay(&store, m.journal_seq, opts.until)?;
                rep.journal_records = r.records;
                rep.journal_books_updated = r.books_updated;
                rep.journal_books_missing = r.books_missing;
                rep.journal_collections = r.collections_updated;
                rep.journal_errors = r.errors;
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(&tmp);
        res?;
        rep.catalogue = CatalogueAction::Restored;
        Ok(())
    }

    /// Applies the journal records after `after_seq` (and not after `until`) to the catalogue in
    /// `store`: the highlights, notes, bookmarks, position, progress and reading status of the
    /// books they name replace what the catalogue has. Books the catalogue does not have are not
    /// created, and no book is ever removed.
    pub fn journal_replay(
        &self,
        store: &SqliteStore,
        after_seq: u64,
        until: Option<DateTime<Utc>>,
    ) -> Result<ReplayReport> {
        let mut rep = ReplayReport::default();
        let mut books: HashMap<u64, bookx_core::library::Book> =
            store.all_books()?.into_iter().map(|b| (b.id, b)).collect();
        let by_file: HashMap<String, u64> =
            books.values().map(|b| (b.file.clone(), b.id)).collect();
        let mut touched: HashSet<u64> = HashSet::new();
        let mut statuses: HashMap<u64, String> = HashMap::new();
        let mut collections: Option<Vec<super::manifest::CollectionState>> = None;
        for (seq, at, _) in self.list_journal()? {
            if seq <= after_seq || until.is_some_and(|u| at > u) {
                continue;
            }
            let rec = match self.read_journal(seq, at) {
                Ok(r) => r,
                Err(e) => {
                    rep.errors.push(format!("record {seq}: {e:#}"));
                    continue;
                }
            };
            rep.records += 1;
            if let Some(c) = rec.collections {
                collections = Some(c);
            }
            for n in rec.books {
                let id = match books.get(&n.id) {
                    Some(b) if n.file.is_empty() || b.file == n.file => Some(n.id),
                    _ => by_file.get(&n.file).copied(),
                };
                let Some(id) = id else {
                    rep.books_missing += 1;
                    continue;
                };
                let b = books.get_mut(&id).expect("id came from the map");
                b.highlights = n.highlights;
                b.bookmarks = n.bookmarks;
                b.position = n.position;
                b.progress = n.progress;
                b.last_read = n.last_read;
                touched.insert(id);
                match n.status {
                    Some(s) => statuses.insert(id, s),
                    None => statuses.remove(&id),
                };
            }
        }
        let changed: Vec<bookx_core::library::Book> =
            touched.iter().filter_map(|id| books.remove(id)).collect();
        rep.books_updated = changed.len();
        if !changed.is_empty() {
            store.put_many(&changed)?;
        }
        for (id, s) in statuses {
            if let Some(st) = ReadingStatus::parse(&s) {
                store
                    .set_status(id, st)
                    .map_err(|e| anyhow!("status of book {id}: {e:#}"))?;
            }
        }
        if let Some(cols) = collections {
            rep.collections_updated =
                apply_collections(store, &cols).map_err(|e| anyhow!("collections: {e:#}"))?;
        }
        Ok(rep)
    }
}

/// Makes the collections of `store` hold the books `cols` names (a collection that is missing
/// is created; one the record does not name is left alone; books the catalogue lacks are
/// ignored). The newest journal record that carries collections is the one applied.
fn apply_collections(
    store: &SqliteStore,
    cols: &[super::manifest::CollectionState],
) -> Result<usize> {
    let existing = store.list_collections()?;
    let mut n = 0;
    for c in cols.iter().take(100_000) {
        let name = c.name.trim();
        if name.is_empty() || name.len() > 1024 {
            continue;
        }
        let id = match existing.iter().find(|e| e.name.eq_ignore_ascii_case(name)) {
            Some(e) => e.id,
            None => store.create_collection(name)?,
        };
        let want: HashSet<u64> = c.books.iter().copied().collect();
        let have: HashSet<u64> = store.collection_books(id).into_iter().collect();
        for b in want.difference(&have) {
            // a book the catalogue does not have is not an error here
            let _ = store.add_to_collection(id, *b);
        }
        for b in have.difference(&want) {
            store.remove_from_collection(id, *b)?;
        }
        n += 1;
    }
    Ok(n)
}

struct CancelWrite<'a, W: std::io::Write> {
    inner: &'a mut W,
    cancel: &'a AtomicBool,
}

impl<W: std::io::Write> std::io::Write for CancelWrite<'_, W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("cancelled"));
        }
        self.inner.write_all(b)?;
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
