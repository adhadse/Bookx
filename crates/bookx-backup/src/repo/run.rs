//! Making a backup: the plan, the run, the journal of changed notes, and retention.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use bookx_store::health::{self, SnapshotKind};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::manifest::{
    journal_name, BlobRef, BookNotes, FileEntry, JournalRecord, Manifest, Role, SnapName, FORMAT,
};
use super::seal::{ids_equal, IdHasher};
use super::source::{
    scan_library, CachedFile, LibrarySource, LocalState, Scan, ScannedFile, Scope, Skipped,
};
use super::store::RepoStore;
use super::{blob_path, Repository, RolledBack};
use crate::push::Cancelled;

const CHANGED: &str = "the file changed while it was being copied";

/// Where a run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Scanning,
    Hashing,
    Uploading,
    Finishing,
    Pruning,
    Done,
}

impl Phase {
    pub fn label(&self) -> &'static str {
        match self {
            Phase::Scanning => "Looking at your library",
            Phase::Hashing => "Finding what is new",
            Phase::Uploading => "Copying new files",
            Phase::Finishing => "Writing the snapshot",
            Phase::Pruning => "Removing old backups",
            Phase::Done => "Done",
        }
    }
}

/// Progress of a run, for the window or the command line.
#[derive(Debug, Clone)]
pub struct RunProgress {
    pub phase: Phase,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// The file being worked on.
    pub current: String,
    pub elapsed: Duration,
    /// Time left, when enough has been done to say.
    pub eta: Option<Duration>,
}

/// How a run is done.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// A manual snapshot is never pruned.
    pub manual: bool,
    pub scope: Scope,
    /// How many automatic snapshots the place keeps (see `health::retention`).
    pub keep: usize,
    pub now: DateTime<Utc>,
    /// Files handled at the same time (hashing and uploading), 1 to 8.
    pub jobs: usize,
    /// Write a snapshot even when nothing changed.
    pub force: bool,
    /// Go on although the place is older than what this library saw before.
    pub accept_rollback: bool,
    /// Remove blobs no snapshot uses after this long; `None` leaves that to `gc`.
    pub gc_grace: Option<chrono::Duration>,
    /// Read every uploaded object back and compare its SHA-256.
    pub verify_writes: bool,
    /// Free space to leave in the place besides what the run needs.
    pub free_margin: u64,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            manual: false,
            scope: Scope::default(),
            keep: 10,
            now: Utc::now(),
            jobs: 3,
            force: false,
            accept_rollback: false,
            gc_grace: Some(chrono::Duration::hours(24)),
            verify_writes: true,
            free_margin: 16 * 1024 * 1024,
        }
    }
}

/// What a run did.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The snapshot written, `None` when nothing had changed.
    pub snapshot: Option<String>,
    pub seq: u64,
    /// Nothing had changed since the last snapshot: nothing was written.
    pub up_to_date: bool,
    pub files: usize,
    pub total_bytes: u64,
    pub new_files: usize,
    /// Content bytes sent, and bytes stored in the place (bigger when encrypted).
    pub new_bytes: u64,
    pub stored_bytes: u64,
    pub reused_files: usize,
    /// Files left out (unreadable, changed while copying, name not allowed).
    pub skipped: Vec<Skipped>,
    /// Files above the size limit.
    pub too_big: Vec<String>,
    pub catalogue_sent: bool,
    pub journal_records: usize,
    pub pruned_snapshots: usize,
    pub gc: Option<super::maint::GcReport>,
    pub verified: bool,
    pub elapsed: Duration,
}

/// What a run would do, without doing it.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub total_files: usize,
    pub total_bytes: u64,
    /// Files whose content the place does not have.
    pub new_files: usize,
    pub new_bytes: u64,
    /// What `new_bytes` take in the place (encryption adds a little).
    pub upload_bytes: u64,
    pub reused_files: usize,
    pub skipped: Vec<Skipped>,
    pub too_big: Vec<String>,
    /// The size of the catalogue (always copied; it changes whenever anything does).
    pub catalogue_bytes: u64,
    pub free: Option<u64>,
    /// `false` when the place has less free space than the run needs.
    pub enough_space: bool,
    /// The place has no repository or no blobs yet: the first run uploads everything.
    pub first_run: bool,
    /// New files were counted from sizes without hashing (the place is empty, so all are new).
    pub estimated: bool,
}

/// Result of [`Repository::journal_append`].
#[derive(Debug, Clone, Default)]
pub struct JournalAppend {
    /// The record written, `None` when nothing changed.
    pub seq: Option<u64>,
    pub books: usize,
    /// The record carries the collections (a shelf was made, renamed, filled or removed).
    pub collections: bool,
}

// -- plumbing --------------------------------------------------------------------------------------------

pub(crate) fn parallel_for<F: Fn(usize) + Sync>(n: usize, jobs: usize, f: F) {
    let next = AtomicUsize::new(0);
    let jobs = jobs.clamp(1, 8).min(n.max(1));
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                f(i);
            });
        }
    });
}

pub(crate) struct Reporter<'a> {
    cb: &'a (dyn Fn(&RunProgress) + Sync),
    start: Instant,
    last: Mutex<Instant>,
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    phase: Mutex<Phase>,
    current: Mutex<String>,
}

impl<'a> Reporter<'a> {
    pub fn new(cb: &'a (dyn Fn(&RunProgress) + Sync)) -> Reporter<'a> {
        Reporter {
            cb,
            start: Instant::now(),
            last: Mutex::new(Instant::now() - Duration::from_secs(10)),
            files_done: AtomicU64::new(0),
            files_total: AtomicU64::new(0),
            bytes_done: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            phase: Mutex::new(Phase::Scanning),
            current: Mutex::new(String::new()),
        }
    }

    pub fn phase(&self, p: Phase, files: u64, bytes: u64) {
        *self.phase.lock().unwrap() = p;
        self.files_done.store(0, Ordering::Relaxed);
        self.bytes_done.store(0, Ordering::Relaxed);
        self.files_total.store(files, Ordering::Relaxed);
        self.bytes_total.store(bytes, Ordering::Relaxed);
        self.emit(true);
    }

    pub fn current(&self, name: &str) {
        *self.current.lock().unwrap() = name.to_string();
    }

    pub fn emit(&self, force: bool) {
        {
            let mut l = self.last.lock().unwrap();
            if !force && l.elapsed() < Duration::from_millis(200) {
                return;
            }
            *l = Instant::now();
        }
        let done = self.bytes_done.load(Ordering::Relaxed);
        let total = self.bytes_total.load(Ordering::Relaxed);
        let elapsed = self.start.elapsed();
        let eta = (done > 0 && total > done && elapsed > Duration::from_secs(2)).then(|| {
            Duration::from_secs_f64(elapsed.as_secs_f64() * (total - done) as f64 / done as f64)
        });
        (self.cb)(&RunProgress {
            phase: *self.phase.lock().unwrap(),
            files_done: self.files_done.load(Ordering::Relaxed),
            files_total: self.files_total.load(Ordering::Relaxed),
            bytes_done: done,
            bytes_total: total,
            current: self.current.lock().unwrap().clone(),
            elapsed,
            eta,
        });
    }
}

fn cancelled_err() -> anyhow::Error {
    anyhow::Error::new(Cancelled)
}

/// A reader that hashes, counts, reports and honours cancel.
struct Tap<'a, R> {
    inner: R,
    id: Option<Arc<Mutex<Option<IdHasher>>>>,
    sha: Option<Arc<Mutex<Sha256>>>,
    cancel: &'a AtomicBool,
    on_bytes: Option<&'a (dyn Fn(u64) + Sync)>,
    source_failed: Option<Arc<AtomicBool>>,
}

impl<R: Read> Read for Tap<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("cancelled"));
        }
        let n = match self.inner.read(buf) {
            Ok(n) => n,
            Err(e) => {
                if let Some(f) = &self.source_failed {
                    f.store(true, Ordering::Relaxed);
                }
                return Err(e);
            }
        };
        if let Some(h) = &self.id {
            if let Some(h) = h.lock().unwrap().as_mut() {
                h.update(&buf[..n]);
            }
        }
        if let Some(s) = &self.sha {
            s.lock().unwrap().update(&buf[..n]);
        }
        if let Some(f) = self.on_bytes {
            f(n as u64);
        }
        Ok(n)
    }
}

struct ShaSink {
    sha: Sha256,
    n: u64,
}

impl Write for ShaSink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.sha.update(b);
        self.n += b.len() as u64;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Why one blob could not be stored.
pub(crate) enum BlobFail {
    /// The file could not be read, or changed under us: leave this file out.
    Source(String),
    Cancelled,
    /// The place failed: the run stops.
    Remote(anyhow::Error),
}

impl BlobFail {
    fn into_error(self) -> anyhow::Error {
        match self {
            BlobFail::Source(s) => anyhow!("{s}"),
            BlobFail::Cancelled => cancelled_err(),
            BlobFail::Remote(e) => e,
        }
    }
}

/// Which blobs the place has, loaded one prefix folder at a time.
struct BlobIndex<'a> {
    store: &'a dyn RepoStore,
    dirs: Mutex<HashMap<String, Arc<HashSet<String>>>>,
}

impl<'a> BlobIndex<'a> {
    fn new(store: &'a dyn RepoStore) -> Self {
        BlobIndex {
            store,
            dirs: Mutex::new(HashMap::new()),
        }
    }

    fn has(&self, id: &str) -> Result<bool> {
        let prefix = &id[..2];
        let set = {
            let mut m = self.dirs.lock().unwrap();
            if let Some(s) = m.get(prefix) {
                s.clone()
            } else {
                let names: HashSet<String> = self
                    .store
                    .list(&format!("blobs/{prefix}"))?
                    .into_iter()
                    .filter(|e| !e.is_dir)
                    .map(|e| e.name)
                    .collect();
                let s = Arc::new(names);
                m.insert(prefix.to_string(), s.clone());
                s
            }
        };
        Ok(set.contains(id))
    }
}

struct Hashed {
    id: String,
    size: u64,
}

struct NewBlob {
    id: String,
    size: u64,
    /// The first file with this content.
    file: usize,
}

struct Prepared {
    scan: Scan,
    hashed: Vec<Option<Hashed>>,
    new: Vec<NewBlob>,
    skipped: Vec<Skipped>,
    reused: usize,
}

impl Repository {
    // -- hashing and uploading one blob ------------------------------------------------------------------

    /// Hashes a file's content into an id (streaming).
    fn hash_file(
        &self,
        f: &ScannedFile,
        cancel: &AtomicBool,
        on_bytes: &(dyn Fn(u64) + Sync),
    ) -> Result<Hashed, String> {
        let mut file = std::fs::File::open(&f.abs).map_err(|e| format!("cannot read it: {e}"))?;
        let mut h = self.crypto.id_hasher();
        let mut buf = vec![0u8; 256 * 1024];
        let mut size = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    h.update(&buf[..n]);
                    size += n as u64;
                    on_bytes(n as u64);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("cannot read it: {e}")),
            }
        }
        Ok(Hashed {
            id: h.finish_hex(),
            size,
        })
    }

    /// Stores the content of `source` (exactly `size` bytes) as blob `id`; the content hashes to
    /// `id` or the blob is removed again. Reads the object back and compares when `verify`.
    pub(crate) fn upload_blob(
        &self,
        id: &str,
        size: u64,
        source: Box<dyn Read + '_>,
        verify: bool,
        cancel: &AtomicBool,
        on_bytes: &(dyn Fn(u64) + Sync),
    ) -> Result<u64, BlobFail> {
        let path = blob_path(id);
        let src_failed = Arc::new(AtomicBool::new(false));
        let id_state = Arc::new(Mutex::new(Some(self.crypto.id_hasher())));
        let plain = Tap {
            inner: source,
            id: Some(id_state.clone()),
            sha: None,
            cancel,
            on_bytes: Some(on_bytes),
            source_failed: Some(src_failed.clone()),
        };
        let sealed = self
            .crypto
            .seal_reader(&path, plain, size)
            .map_err(BlobFail::Remote)?;
        let sent = Arc::new(Mutex::new(Sha256::new()));
        let mut wire = Tap {
            inner: sealed,
            id: None,
            sha: Some(sent.clone()),
            cancel,
            on_bytes: None,
            source_failed: None,
        };
        let stored = self.crypto.stored_len(size);
        if let Err(e) = self.store.put(&path, &mut wire, stored, cancel) {
            if cancel.load(Ordering::Relaxed) || e.downcast_ref::<Cancelled>().is_some() {
                return Err(BlobFail::Cancelled);
            }
            let text = format!("{e:#}");
            if src_failed.load(Ordering::Relaxed) || text.contains(CHANGED) {
                return Err(BlobFail::Source(text));
            }
            return Err(BlobFail::Remote(e));
        }
        let got = id_state
            .lock()
            .unwrap()
            .take()
            .expect("hasher present")
            .finish_hex();
        if !ids_equal(&got, id) {
            let _ = self.store.delete(&path);
            return Err(BlobFail::Source(CHANGED.into()));
        }
        if verify {
            let mut sink = ShaSink {
                sha: Sha256::new(),
                n: 0,
            };
            let back = self.store.get_to(&path, &mut sink, stored);
            let want: [u8; 32] = sent.lock().unwrap().clone().finalize().into();
            let ok = back.is_ok() && sink.n == stored && sink.sha.finalize().as_slice() == want;
            if !ok {
                let _ = self.store.delete(&path);
                return Err(BlobFail::Remote(match back {
                    Err(e) => anyhow!("could not read the copy back: {e:#}"),
                    Ok(_) => anyhow!("the copy does not match the original (it was removed); the place may be full or unreliable"),
                }));
            }
        }
        Ok(stored)
    }

    // -- preparing ---------------------------------------------------------------------------------------

    /// Scans, hashes what is not cached and finds which contents the place lacks.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &self,
        books: &[bookx_core::library::Book],
        root: &Path,
        scope: &Scope,
        state: &mut LocalState,
        jobs: usize,
        cancel: &AtomicBool,
        rep: &Reporter,
        assume_empty: bool,
    ) -> Result<Prepared> {
        rep.phase(Phase::Scanning, 0, 0);
        let scan = scan_library(root, books, scope)?;
        let files = &scan.files;
        let total_bytes: u64 = files.iter().map(|f| f.size).sum();
        rep.phase(Phase::Hashing, files.len() as u64, total_bytes);

        let results: Mutex<Vec<Option<Result<Hashed, String>>>> =
            Mutex::new((0..files.len()).map(|_| None).collect());
        let index = BlobIndex::new(self.store.as_ref());
        let exists: Mutex<Vec<Option<bool>>> = Mutex::new(vec![None; files.len()]);
        let remote_err: Mutex<Option<anyhow::Error>> = Mutex::new(None);
        let state_ro = &*state;
        parallel_for(files.len(), jobs, |i| {
            if cancel.load(Ordering::Relaxed) || remote_err.lock().unwrap().is_some() {
                return;
            }
            let f = &files[i];
            rep.current(&f.rel);
            let r = match state_ro.cached(f) {
                Some(id) => {
                    rep.bytes_done.fetch_add(f.size, Ordering::Relaxed);
                    Ok(Hashed {
                        id: id.to_string(),
                        size: f.size,
                    })
                }
                None => self.hash_file(f, cancel, &|n| {
                    rep.bytes_done.fetch_add(n, Ordering::Relaxed);
                    rep.emit(false);
                }),
            };
            if let Ok(h) = &r {
                if !assume_empty {
                    match index.has(&h.id) {
                        Ok(b) => exists.lock().unwrap()[i] = Some(b),
                        Err(e) => {
                            *remote_err.lock().unwrap() = Some(e);
                            return;
                        }
                    }
                }
            }
            rep.files_done.fetch_add(1, Ordering::Relaxed);
            rep.emit(false);
            results.lock().unwrap()[i] = Some(r);
        });
        let results = results.into_inner().unwrap();
        // what was hashed is worth remembering even when the run stops here
        let present: HashSet<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        state.files.retain(|k, _| present.contains(k.as_str()));
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i128)
            .unwrap_or(0);
        for (f, r) in files.iter().zip(&results) {
            if let Some(Ok(h)) = r {
                // a file changed in the last two seconds could still change within the same
                // timestamp: it is read again next time instead of trusted
                if h.size == f.size && now_ns - f.mtime_ns > 2_000_000_000 {
                    state.files.insert(
                        f.rel.clone(),
                        CachedFile {
                            size: f.size,
                            mtime_ns: f.mtime_ns,
                            blob: h.id.clone(),
                        },
                    );
                }
            }
        }
        if let Some(e) = remote_err.into_inner().unwrap() {
            return Err(e);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled_err());
        }
        let exists = exists.into_inner().unwrap();
        let mut skipped = Vec::new();
        let mut hashed = Vec::new();
        let mut new: Vec<NewBlob> = Vec::new();
        let mut seen_new: HashSet<String> = HashSet::new();
        let mut reused = 0;
        for (i, r) in results.into_iter().enumerate() {
            match r {
                Some(Ok(h)) => {
                    let present = !assume_empty && exists[i] == Some(true);
                    if present {
                        reused += 1;
                    } else if seen_new.insert(h.id.clone()) {
                        new.push(NewBlob {
                            id: h.id.clone(),
                            size: h.size,
                            file: i,
                        });
                    } else {
                        reused += 1; // the same content as an earlier file
                    }
                    hashed.push(Some(h));
                }
                Some(Err(why)) => {
                    skipped.push(Skipped {
                        path: files[i].rel.clone(),
                        reason: why,
                    });
                    hashed.push(None);
                }
                None => hashed.push(None),
            }
        }
        Ok(Prepared {
            scan,
            hashed,
            new,
            skipped,
            reused,
        })
    }

    fn blobs_present(&self) -> Result<bool> {
        Ok(self.store.list("blobs")?.iter().any(|e| e.is_dir))
    }

    // -- the plan ----------------------------------------------------------------------------------------

    /// What a run would send, without sending anything. Files already hashed (the library
    /// remembers them) are not read again; when the place has no blobs yet, everything is new and
    /// nothing is hashed (`estimated`).
    pub fn plan(
        &self,
        lib: &LibrarySource,
        scope: &Scope,
        jobs: usize,
        cancel: &AtomicBool,
        progress: &(dyn Fn(&RunProgress) + Sync),
    ) -> Result<Plan> {
        let books = lib.books()?;
        let empty = !self.blobs_present()?;
        let rep = Reporter::new(progress);
        let mut state = LocalState::load(&lib.root, &self.id, &self.crypto.fingerprint());
        if empty {
            return estimate_new(lib, scope, self.is_encrypted(), self.store.free_space());
        }
        let p = self.prepare(
            &books, &lib.root, scope, &mut state, jobs, cancel, &rep, false,
        )?;
        let _ = state.save(&lib.root);
        let total_files = p.scan.files.len();
        let total_bytes: u64 = p.scan.files.iter().map(|f| f.size).sum();
        let new_bytes: u64 = p.new.iter().map(|b| b.size).sum();
        let upload_bytes: u64 = p.new.iter().map(|b| self.crypto.stored_len(b.size)).sum();
        let catalogue = catalogue_size(&lib.root);
        let free = self.store.free_space();
        let mut skipped = p.scan.skipped.clone();
        skipped.extend(p.skipped.clone());
        Ok(Plan {
            total_files,
            total_bytes,
            new_files: p.new.len(),
            new_bytes,
            upload_bytes,
            reused_files: p.reused,
            skipped,
            too_big: p.scan.too_big.clone(),
            catalogue_bytes: catalogue,
            free,
            enough_space: free.is_none_or(|f| f >= upload_bytes + catalogue),
            first_run: false,
            estimated: false,
        })
    }

    // -- the run -----------------------------------------------------------------------------------------

    /// Makes a backup of `lib`: new or changed files, the catalogue and the notes. Only content
    /// the place does not have is sent. The snapshot manifest is written last. Per-file problems
    /// (unreadable, changed while copying) leave that file out and are reported; a problem with
    /// the place stops the run and nothing but unused blobs is left behind.
    pub fn run(
        &self,
        lib: &LibrarySource,
        opts: &RunOptions,
        progress: &(dyn Fn(&RunProgress) + Sync),
        cancel: &AtomicBool,
    ) -> Result<Report> {
        let started = Instant::now();
        let rep = Reporter::new(progress);
        let verify = opts.verify_writes;
        self.store.check()?;
        let _lock = self.lock()?;
        // continuing a half-sent big file needs identical bytes (plain objects) and the read-back
        self.store
            .allow_resume(!self.crypto.is_encrypted() && verify);
        let mut state = LocalState::load(&lib.root, &self.id, &self.crypto.fingerprint());
        let existing = self.list_snapshots()?;
        let remote_max = existing.first().map(|s| s.name.seq).unwrap_or(0);
        if remote_max < state.last_seen_seq && !opts.accept_rollback {
            return Err(anyhow::Error::new(RolledBack {
                seen: state.last_seen_seq,
                found: remote_max,
            }));
        }
        let books = lib.books()?;

        // 1. the notes that changed, as a journal record (cheap, and first: they matter most)
        let journal = self.journal_with(lib, &books, &mut state, opts.now, cancel)?;

        // 2. what is new
        let remote_empty = !self.blobs_present()?;
        let prep = self.prepare(
            &books,
            &lib.root,
            &opts.scope,
            &mut state,
            opts.jobs,
            cancel,
            &rep,
            remote_empty,
        );
        let prep = match prep {
            Ok(p) => p,
            Err(e) => {
                let _ = state.save(&lib.root);
                return Err(e);
            }
        };

        // 3. the catalogue and the notes, as local temporary copies
        let tmp = lib.tmp_dir()?;
        let cat_path = tmp.join(format!("catalogue-{}.db", super::seal::random_hex(4)?));
        let cleanup = || {
            let _ = std::fs::remove_file(&cat_path);
        };
        lib.store.backup_to(&cat_path)?;
        let (cat_id, cat_size) = {
            let f = ScannedFile {
                rel: String::new(),
                abs: cat_path.clone(),
                size: 0,
                mtime: 0,
                mtime_ns: 0,
                role: Role::Book,
            };
            match self.hash_file(&f, cancel, &|_| {}) {
                Ok(h) => (h.id, h.size),
                Err(e) => {
                    cleanup();
                    if cancel.load(Ordering::Relaxed) {
                        return Err(cancelled_err());
                    }
                    bail!("cannot read the catalogue copy: {e}");
                }
            }
        };
        let lib_state = lib.library_state(&books)?;
        let state_bytes = serde_json::to_vec(&lib_state)?;
        let state_id = self.crypto.id_of(&state_bytes);

        // 4. nothing changed?
        let latest = existing.first().and_then(|s| self.manifest(&s.name).ok());
        let mut files: Vec<FileEntry> = Vec::new();
        for (f, h) in prep.scan.files.iter().zip(&prep.hashed) {
            if let Some(h) = h {
                files.push(FileEntry {
                    path: f.rel.clone(),
                    blob: h.id.clone(),
                    size: h.size,
                    mtime: f.mtime,
                    role: f.role,
                });
            }
        }
        let seq = remote_max.max(state.last_seen_seq) + 1;
        let created = DateTime::from_timestamp(opts.now.timestamp(), 0).unwrap_or(opts.now);
        let mut manifest = Manifest {
            format: FORMAT,
            repo: self.id.clone(),
            seq,
            created,
            kind: if opts.manual { "manual" } else { "auto" }.into(),
            app: format!("Bookx {}", env!("CARGO_PKG_VERSION")),
            books: books.len(),
            catalogue: BlobRef {
                blob: cat_id.clone(),
                size: cat_size,
            },
            state: BlobRef {
                blob: state_id.clone(),
                size: state_bytes.len() as u64,
            },
            journal_seq: state.last_journal_seq,
            include_books: opts.scope.include_books,
            include_covers: opts.scope.include_covers,
            too_big: prep.scan.too_big.clone(),
            files,
        };
        let mut report = Report {
            files: manifest.files.len(),
            total_bytes: manifest.total_bytes(),
            too_big: prep.scan.too_big.clone(),
            journal_records: usize::from(journal.seq.is_some()),
            verified: verify,
            ..Report::default()
        };
        report.skipped = prep.scan.skipped.clone();
        report.skipped.extend(prep.skipped.clone());
        if !opts.force
            && prep.new.is_empty()
            && latest.as_ref().is_some_and(|l| l.same_content(&manifest))
        {
            cleanup();
            let _ = state.save(&lib.root);
            report.up_to_date = true;
            report.seq = remote_max;
            report.reused_files = prep.reused;
            report.snapshot = None;
            report.elapsed = started.elapsed();
            rep.phase(Phase::Done, 0, 0);
            return Ok(report);
        }

        // 5. enough room?
        let upload: u64 = prep
            .new
            .iter()
            .map(|b| self.crypto.stored_len(b.size))
            .sum::<u64>()
            + self.crypto.stored_len(cat_size)
            + self.crypto.stored_len(state_bytes.len() as u64);
        if let Some(free) = self.store.free_space() {
            if free < upload.saturating_add(opts.free_margin) {
                cleanup();
                let _ = state.save(&lib.root);
                bail!(
                    "not enough space in the backup place: {} needed, {} free",
                    human(upload),
                    human(free)
                );
            }
        }

        // 6. upload the new blobs
        let new_total: u64 = prep.new.iter().map(|b| b.size).sum();
        rep.phase(Phase::Uploading, prep.new.len() as u64, new_total);
        let failed_ids: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
        let skipped_src: Mutex<Vec<Skipped>> = Mutex::new(Vec::new());
        let fatal: Mutex<Option<BlobFail>> = Mutex::new(None);
        let stored = AtomicU64::new(0);
        let sent_files = AtomicUsize::new(0);
        let sent_bytes = AtomicU64::new(0);
        let abort = AtomicBool::new(false);
        let local_cancel = &abort;
        parallel_for(prep.new.len(), opts.jobs, |i| {
            if cancel.load(Ordering::Relaxed) || local_cancel.load(Ordering::Relaxed) {
                return;
            }
            let b = &prep.new[i];
            let f = &prep.scan.files[b.file];
            rep.current(&f.rel);
            let file = match std::fs::File::open(&f.abs) {
                Ok(x) => x,
                Err(e) => {
                    failed_ids.lock().unwrap().insert(b.id.clone());
                    skipped_src.lock().unwrap().push(Skipped {
                        path: f.rel.clone(),
                        reason: format!("cannot read it: {e}"),
                    });
                    return;
                }
            };
            let res = self.upload_blob(&b.id, b.size, Box::new(file), verify, cancel, &|n| {
                rep.bytes_done.fetch_add(n, Ordering::Relaxed);
                if !self.lock_beat() {
                    local_cancel.store(true, Ordering::Relaxed);
                }
                rep.emit(false);
            });
            match res {
                Ok(st) => {
                    stored.fetch_add(st, Ordering::Relaxed);
                    sent_files.fetch_add(1, Ordering::Relaxed);
                    sent_bytes.fetch_add(b.size, Ordering::Relaxed);
                    rep.files_done.fetch_add(1, Ordering::Relaxed);
                    rep.emit(true);
                }
                Err(BlobFail::Source(why)) => {
                    failed_ids.lock().unwrap().insert(b.id.clone());
                    skipped_src.lock().unwrap().push(Skipped {
                        path: f.rel.clone(),
                        reason: why,
                    });
                }
                Err(BlobFail::Cancelled) => {}
                Err(fail @ BlobFail::Remote(_)) => {
                    local_cancel.store(true, Ordering::Relaxed);
                    let mut g = fatal.lock().unwrap();
                    if g.is_none() {
                        *g = Some(fail);
                    }
                }
            }
            rep.emit(false);
        });
        let _ = state.save(&lib.root);
        if let Some(f) = fatal.into_inner().unwrap() {
            cleanup();
            return Err(f.into_error());
        }
        if cancel.load(Ordering::Relaxed) {
            cleanup();
            return Err(cancelled_err());
        }
        // contents that could not be stored: their files are left out of the snapshot
        let failed = failed_ids.into_inner().unwrap();
        report.skipped.extend(skipped_src.into_inner().unwrap());
        if !failed.is_empty() {
            let before = manifest.files.len();
            let dropped: Vec<String> = manifest
                .files
                .iter()
                .filter(|f| failed.contains(&f.blob))
                .map(|f| f.path.clone())
                .collect();
            manifest.files.retain(|f| !failed.contains(&f.blob));
            for p in dropped {
                if !report.skipped.iter().any(|s| s.path == p) {
                    report.skipped.push(Skipped {
                        path: p,
                        reason: "the same content could not be read".into(),
                    });
                }
            }
            report.files = manifest.files.len();
            report.total_bytes = manifest.total_bytes();
            debug_assert!(manifest.files.len() <= before);
        }

        // 7. catalogue and notes
        rep.phase(Phase::Finishing, 2, cat_size + state_bytes.len() as u64);
        let mut cat_sent = false;
        let r = (|| -> Result<()> {
            if !self.blob_exists(&cat_id)? {
                let f = std::fs::File::open(&cat_path)?;
                self.upload_blob(&cat_id, cat_size, Box::new(f), verify, cancel, &|n| {
                    rep.bytes_done.fetch_add(n, Ordering::Relaxed);
                    rep.emit(false);
                })
                .map(|st| {
                    stored.fetch_add(st, Ordering::Relaxed);
                })
                .map_err(BlobFail::into_error)?;
                cat_sent = true;
            }
            if !self.blob_exists(&state_id)? {
                self.upload_blob(
                    &state_id,
                    state_bytes.len() as u64,
                    Box::new(std::io::Cursor::new(state_bytes.clone())),
                    verify,
                    cancel,
                    &|_| {},
                )
                .map(|st| {
                    stored.fetch_add(st, Ordering::Relaxed);
                })
                .map_err(BlobFail::into_error)?;
            }
            Ok(())
        })();
        cleanup();
        r?;

        // 8. the manifest, last (and only while the place is still ours)
        self.lock_held()?;
        let name = SnapName {
            seq,
            at: created,
            manual: opts.manual,
        };
        let body = serde_json::to_vec(&manifest)?;
        self.put_checked(&name.path(), &body, cancel)?;
        state.last_seen_seq = seq;
        state.save(&lib.root)?;
        report.snapshot = Some(name.file_name());
        report.seq = seq;
        report.new_files = sent_files.load(Ordering::Relaxed);
        report.new_bytes = sent_bytes.load(Ordering::Relaxed);
        report.stored_bytes = stored.load(Ordering::Relaxed);
        report.reused_files = prep.reused;
        report.catalogue_sent = cat_sent;

        // 9. retention and cleanup
        rep.phase(Phase::Pruning, 0, 0);
        match self.prune(opts.keep, opts.now) {
            Ok(n) => report.pruned_snapshots = n,
            Err(e) => report.skipped.push(Skipped {
                path: "(old backups)".into(),
                reason: format!("could not remove old backups: {e:#}"),
            }),
        }
        if let Some(grace) = opts.gc_grace {
            report.gc = self.gc(grace, opts.now, cancel).ok();
        }
        report.elapsed = started.elapsed();
        rep.phase(Phase::Done, 0, 0);
        Ok(report)
    }

    fn blob_exists(&self, id: &str) -> Result<bool> {
        Ok(self.store.stat(&blob_path(id))?.is_some())
    }

    // -- the journal -------------------------------------------------------------------------------------

    /// Writes a small record with the highlights, notes, bookmarks, positions and reading status
    /// of the books that changed since the last record, so they are protected between snapshots.
    /// Cheap enough to call every few minutes; writes nothing when nothing changed.
    pub fn journal_append(
        &self,
        lib: &LibrarySource,
        now: DateTime<Utc>,
        cancel: &AtomicBool,
    ) -> Result<JournalAppend> {
        self.store.check()?;
        let mut state = LocalState::load(&lib.root, &self.id, &self.crypto.fingerprint());
        let books = lib.books()?;
        let r = self.journal_with(lib, &books, &mut state, now, cancel)?;
        state.save(&lib.root)?;
        Ok(r)
    }

    fn journal_with(
        &self,
        lib: &LibrarySource,
        books: &[bookx_core::library::Book],
        state: &mut LocalState,
        now: DateTime<Utc>,
        cancel: &AtomicBool,
    ) -> Result<JournalAppend> {
        let notes = lib.all_notes(books);
        let mut now_map: std::collections::BTreeMap<u64, String> =
            std::collections::BTreeMap::new();
        let mut changed: Vec<BookNotes> = Vec::new();
        for n in &notes {
            let fp = n.fingerprint();
            if state.notes.get(&n.id) != Some(&fp) {
                changed.push(n.clone());
            }
            now_map.insert(n.id, fp);
        }
        // notes that were all removed from a book that still exists
        let live: HashMap<u64, &bookx_core::library::Book> =
            books.iter().map(|b| (b.id, b)).collect();
        for id in state.notes.keys() {
            if !now_map.contains_key(id) {
                if let Some(b) = live.get(id) {
                    changed.push(LibrarySource::notes_of(b, None));
                }
            }
        }
        let cols = lib.collections()?;
        let cols_fp = collections_fingerprint(&cols);
        // the first look at a library only remembers the shelves: a snapshot has them already
        let cols_changed =
            state.collections != cols_fp && !(state.collections.is_empty() && cols.is_empty());
        if changed.is_empty() && !cols_changed {
            state.notes = now_map;
            state.collections = cols_fp;
            return Ok(JournalAppend::default());
        }
        let _lock = self.lock()?;
        let remote_max = self.list_journal()?.last().map(|j| j.0).unwrap_or(0);
        let seq = remote_max.max(state.last_journal_seq) + 1;
        let at = DateTime::from_timestamp(now.timestamp(), 0).unwrap_or(now);
        let rec = JournalRecord {
            format: FORMAT,
            repo: self.id.clone(),
            seq,
            at,
            books: changed.clone(),
            collections: cols_changed.then(|| cols.clone()),
        };
        let path = format!("journal/{}", journal_name(seq, at));
        self.put_checked(&path, &serde_json::to_vec(&rec)?, cancel)
            .context("writing the journal record")?;
        state.last_journal_seq = seq;
        state.notes = now_map;
        state.collections = cols_fp;
        Ok(JournalAppend {
            seq: Some(seq),
            books: changed.len(),
            collections: cols_changed,
        })
    }

    // -- retention ---------------------------------------------------------------------------------------

    /// Applies the retention rule to the snapshots (manual ones stay) and drops journal records
    /// every kept snapshot already contains. Returns how many snapshots were removed.
    pub fn prune(&self, keep: usize, now: DateTime<Utc>) -> Result<usize> {
        let _lock = self.lock()?;
        let all = self.list_snapshots()?;
        let input: Vec<(DateTime<Utc>, SnapshotKind)> = all
            .iter()
            .map(|s| {
                (
                    s.name.at,
                    if s.name.manual {
                        SnapshotKind::Manual
                    } else {
                        SnapshotKind::Auto
                    },
                )
            })
            .collect();
        let doomed = health::retention(&input, keep.max(1), now);
        for &i in &doomed {
            self.store.delete(&all[i].name.path())?;
        }
        self.compact_journal()?;
        Ok(doomed.len())
    }

    /// Removes journal records older than the oldest kept snapshot's catalogue. Only when every
    /// kept snapshot can be read (otherwise nothing is removed).
    pub fn compact_journal(&self) -> Result<usize> {
        let _lock = self.lock()?;
        let snaps = self.list_snapshots()?;
        let Some(oldest) = snaps.last() else {
            return Ok(0);
        };
        let upto = self.manifest(&oldest.name)?.journal_seq;
        let mut n = 0;
        for (seq, at, _) in self.list_journal()? {
            if seq <= upto {
                self.store
                    .delete(&format!("journal/{}", journal_name(seq, at)))?;
                n += 1;
            }
        }
        Ok(n)
    }
}

/// The plan for a place that has no repository yet: everything is new (counted from sizes, nothing
/// is read or hashed), so it is quick on any library. `encrypted` adds the small overhead of
/// encryption to the stored size; `free` is what the place reports.
pub fn estimate_new(
    lib: &LibrarySource,
    scope: &Scope,
    encrypted: bool,
    free: Option<u64>,
) -> Result<Plan> {
    let books = lib.books()?;
    let scan = scan_library(&lib.root, &books, scope)?;
    let total: u64 = scan.files.iter().map(|f| f.size).sum();
    let stored = |n: u64| {
        if encrypted {
            super::seal::sealed_len(n)
        } else {
            n
        }
    };
    let upload_bytes: u64 = scan.files.iter().map(|f| stored(f.size)).sum();
    let catalogue_bytes = catalogue_size(&lib.root);
    Ok(Plan {
        total_files: scan.files.len(),
        total_bytes: total,
        new_files: scan.files.len(),
        new_bytes: total,
        upload_bytes,
        reused_files: 0,
        skipped: scan.skipped,
        too_big: scan.too_big,
        catalogue_bytes,
        free,
        enough_space: free.is_none_or(|f| f >= upload_bytes + catalogue_bytes),
        first_run: true,
        estimated: true,
    })
}

fn collections_fingerprint(c: &[super::manifest::CollectionState]) -> String {
    super::seal::hex(&Sha256::digest(serde_json::to_vec(c).unwrap_or_default()))
}

fn catalogue_size(root: &Path) -> u64 {
    std::fs::metadata(root.join(bookx_store::LIBRARY_DB))
        .map(|m| m.len())
        .unwrap_or(0)
}

pub(crate) fn human(b: u64) -> String {
    bookx_serve::devfs::human_size(b)
}
