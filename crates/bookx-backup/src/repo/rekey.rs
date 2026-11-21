//! Changing the passphrase of an encrypted repository: every object is re-encrypted in place under
//! keys derived from the new passphrase, safely and resumably.
//!
//! * The *id key* (the key of the keyed blob ids, which are object names) is kept: it is sealed
//!   under the new keys in `bookx-repo.json`, so no object is renamed and no snapshot is rewritten.
//!   The *encryption* key is new: after the change the old passphrase opens nothing that was
//!   re-encrypted. Somebody who still holds the old passphrase can only confirm that the place
//!   has a file they already own (what the keyed ids always allowed to a passphrase holder).
//! * `rekey.json` is written first (new key parameters, a check value, nothing secret). Each object
//!   is read, opened with the old keys, sealed with the new ones (same path: the path is part of
//!   the authenticated data) and written through the store's atomic `put`, then read back. An
//!   object that already opens with the new keys is skipped, so a run that was cut off
//!   (power, network, cancel) continues where it stopped: run the same change again with the same
//!   two passphrases.
//! * The commit is one atomic write of `bookx-repo.json` (new key parameters, check value, sealed
//!   id key, the id of this change). Until then the old settings are in place and the repository
//!   refuses to open with [`RekeyPending`] (some objects are under the new keys); afterwards the
//!   marker is deleted. A marker left behind by a crash between the two steps is recognised by its
//!   id and ignored.
//! * Writers are shut out by the repository lock for the whole time.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::manifest::FORMAT;
use super::run::parallel_for;
use super::seal::{self, Crypto, IdHasher, Keys};
use super::store::RepoStore;
use super::{KdfConfig, RepoConfig, Repository, Secrets, CHECK_TEXT, CONFIG, MAX_CONFIG_BYTES};
use crate::crypto::{self, Kdf};
use crate::push::Cancelled;

/// The marker file of a change in progress.
pub const MARKER: &str = "rekey.json";

/// The repository is half way through a passphrase change.
#[derive(Debug)]
pub struct RekeyPending;

impl std::fmt::Display for RekeyPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a backup passphrase change was interrupted. Run Change Backup Passphrase again with the old and the new passphrase to finish it before using this backup")
    }
}
impl std::error::Error for RekeyPending {}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Marker {
    format: u32,
    repo: String,
    id: String,
    started: String,
    kdf: KdfConfig,
    /// The check text sealed under the new keys (proves the new passphrase when resuming).
    check: String,
    /// The id key sealed under the new keys.
    id_key: String,
}

/// How a change is done.
#[derive(Debug, Clone)]
pub struct RekeyOptions {
    /// Key derivation cost of the new passphrase.
    pub kdf: Kdf,
    /// Read every rewritten object back and open it with the new keys.
    pub verify: bool,
    /// Objects handled at the same time (1 to 8).
    pub jobs: usize,
}

impl Default for RekeyOptions {
    fn default() -> Self {
        RekeyOptions {
            kdf: Kdf::DEFAULT,
            verify: true,
            jobs: 2,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RekeyProgress {
    pub objects_done: usize,
    pub objects_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
}

#[derive(Debug, Clone, Default)]
pub struct RekeyReport {
    pub objects: usize,
    /// Objects rewritten in this call.
    pub converted: usize,
    /// Objects that were under the new keys already (an earlier, cut-off call).
    pub already: usize,
    pub bytes: u64,
    /// This call finished a change an earlier one started.
    pub resumed: bool,
    /// The change was committed in an earlier call; only the marker was left.
    pub was_done: bool,
}

/// Called by `open`: a marker that does not belong to the settings in place means that some
/// objects are under keys the settings do not describe.
pub(super) fn refuse_unfinished(store: &dyn RepoStore, cfg: &RepoConfig) -> Result<()> {
    if !cfg.encrypted || store.stat(MARKER)?.is_none() {
        return Ok(());
    }
    match read_marker(store) {
        Ok(m) if cfg.rekeyed.as_deref() == Some(m.id.as_str()) => Ok(()),
        _ => Err(anyhow::Error::new(RekeyPending)),
    }
}

fn read_marker(store: &dyn RepoStore) -> Result<Marker> {
    let b = store.get_bytes(MARKER, MAX_CONFIG_BYTES)?;
    let m: Marker =
        serde_json::from_slice(&b).context("the passphrase change record is damaged")?;
    if m.format != FORMAT || m.id.len() != 32 {
        bail!("the passphrase change record is damaged");
    }
    Ok(m)
}

fn read_config(store: &dyn RepoStore) -> Result<RepoConfig> {
    if store.stat(CONFIG)?.is_none() {
        return Err(anyhow::Error::new(super::NoRepository));
    }
    let b = store.get_bytes(CONFIG, MAX_CONFIG_BYTES)?;
    serde_json::from_slice(&b).context("the repository settings (bookx-repo.json) are damaged")
}

fn derive_new(pass: &str, kdf: &KdfConfig, old: &Keys) -> Result<Crypto> {
    if kdf.algorithm != "argon2id"
        || !crypto::kdf_is_reasonable(kdf.memory_kib, kdf.passes, kdf.lanes)
    {
        bail!("the passphrase change record asks for unreasonable key settings");
    }
    let salt = seal::unhex(&kdf.salt)?;
    if salt.len() < 16 || salt.len() > 64 {
        bail!("the passphrase change record is damaged");
    }
    let mut master = crypto::derive(
        pass,
        &salt,
        Kdf {
            memory_kib: kdf.memory_kib,
            passes: kdf.passes,
            lanes: kdf.lanes,
        },
    )?;
    let k = Keys::from_master_keeping_ids(&master, old);
    zeroize::Zeroize::zeroize(&mut master);
    Ok(Crypto::Keyed(k))
}

// -- a pipe between the opening and the sealing of one object -------------------------------------------

struct ChanWriter {
    tx: SyncSender<Vec<u8>>,
    buf: Vec<u8>,
    hasher: Option<IdHasher>,
}

impl Write for ChanWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if let Some(h) = &mut self.hasher {
            h.update(b);
        }
        self.buf.extend_from_slice(b);
        if self.buf.len() >= 64 * 1024 {
            self.flush()?;
        }
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            self.tx
                .send(std::mem::take(&mut self.buf))
                .map_err(|_| std::io::Error::other("the writer stopped"))?;
        }
        Ok(())
    }
}

struct ChanReader {
    rx: Receiver<Vec<u8>>,
    cur: Vec<u8>,
    pos: usize,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.cur.len() {
            match self.rx.recv() {
                Ok(v) => {
                    self.cur = v;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.cur.len() - self.pos);
        out[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Counts what is written; refuses everything after the first byte.
struct Sniff {
    got: bool,
}

impl Write for Sniff {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if !b.is_empty() {
            self.got = true;
        }
        Err(std::io::Error::other("sniffed"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Whether the object at `path` opens with `crypto` (only the start is read for big objects).
fn opens_with(store: &dyn RepoStore, crypto: &Crypto, path: &str, stored: u64) -> bool {
    let first = (seal::OBJ_HEADER + 2 * (seal::CHUNK + 16)) as u64;
    if stored <= first {
        let Some(plain) = seal::plain_len(stored) else {
            return false;
        };
        return match store.get_bytes(path, stored) {
            Ok(b) => crypto.open_bytes(path, &b, plain).is_ok(),
            Err(_) => false,
        };
    }
    let mut s = Sniff { got: false };
    {
        let mut w = crypto.open_writer(path, &mut s, u64::MAX);
        // the read stops when the first chunk opened (the sniffer refuses further data)
        let _ = store.get_to(path, &mut w, stored);
    }
    s.got
}

/// Opens `path` with `old`, seals it with `new`, replaces it. Returns the content size.
fn convert(
    store: &dyn RepoStore,
    old: &Crypto,
    new: &Crypto,
    path: &str,
    stored: u64,
    cancel: &AtomicBool,
) -> Result<u64> {
    let plain = seal::plain_len(stored).ok_or_else(|| anyhow!("{path} has an impossible size"))?;
    // a blob's name is the keyed hash of its content: check it while passing through
    let blob_id = path
        .strip_prefix("blobs/")
        .and_then(|r| r.rsplit('/').next())
        .filter(|n| seal::valid_id(n))
        .map(str::to_string);
    let (tx, rx) = sync_channel::<Vec<u8>>(4);
    let (producer, consumer) = std::thread::scope(|s| {
        let prod = s.spawn(move || -> Result<()> {
            let mut cw = ChanWriter {
                tx,
                buf: Vec::new(),
                hasher: blob_id.as_ref().map(|_| old.id_hasher()),
            };
            {
                let mut ow = old.open_writer(path, &mut cw, plain);
                store
                    .get_to(path, &mut ow, stored)
                    .with_context(|| format!("reading {path}"))?;
                ow.finish()
                    .with_context(|| format!("opening {path} with the old passphrase"))?;
            }
            cw.flush()?;
            if let (Some(id), Some(h)) = (blob_id, cw.hasher.take()) {
                if !seal::ids_equal(&h.finish_hex(), &id) {
                    bail!("{path} does not match its name (damaged or tampered with)");
                }
            }
            Ok(())
        });
        let consumer = (|| -> Result<()> {
            let reader = ChanReader {
                rx,
                cur: Vec::new(),
                pos: 0,
            };
            let mut sealed = new.seal_reader(path, reader, plain)?;
            store.put(path, &mut sealed, new.stored_len(plain), cancel)
        })();
        // the sealer is gone: a producer still sending stops
        let p = prod
            .join()
            .unwrap_or_else(|_| Err(anyhow!("worker panicked")));
        (p, consumer)
    });
    // the producer's reason is the real one (the consumer only saw a short read)
    producer?;
    consumer?;
    Ok(plain)
}

impl Repository {
    /// Changes the passphrase of the encrypted repository in `store` from `old` to `new`,
    /// re-encrypting every object in place. See the module notes for the guarantees. Run it again
    /// with the same arguments after an interruption.
    pub fn change_passphrase(
        store: Box<dyn RepoStore>,
        old: &str,
        new: &str,
        opts: &RekeyOptions,
        cancel: &AtomicBool,
        progress: &(dyn Fn(&RekeyProgress) + Sync),
    ) -> Result<RekeyReport> {
        if new.is_empty() {
            bail!("the new passphrase is empty");
        }
        if old == new {
            bail!("the new passphrase is the same as the old one");
        }
        let cfg = read_config(store.as_ref())?;
        if !cfg.encrypted {
            bail!("this backup is not encrypted; there is no passphrase to change");
        }
        let marker = if store.stat(MARKER)?.is_some() {
            Some(read_marker(store.as_ref())?)
        } else {
            None
        };
        if let Some(m) = &marker {
            if cfg.rekeyed.as_deref() == Some(m.id.as_str()) {
                // committed; only the marker was left. The settings now open with `new`.
                let r = Repository::open_with(store, &Secrets::passphrase(new), false)
                    .map_err(|_| anyhow!("wrong passphrase for this backup"))?;
                r.store.delete(MARKER)?;
                return Ok(RekeyReport {
                    was_done: true,
                    ..RekeyReport::default()
                });
            }
        }
        // the new passphrase already opens the settings: an earlier call (or another place of the
        // same account) did the change
        if marker.is_none() && super::crypto_for(&cfg, &Secrets::passphrase(new))?.is_some() {
            return Ok(RekeyReport {
                was_done: true,
                ..RekeyReport::default()
            });
        }
        let repo = Repository::open_with(store, &Secrets::passphrase(old), true)
            .map_err(|e| anyhow!("the old passphrase does not open this backup: {e:#}"))?;
        let _lock = repo.lock()?;
        let old_keys = repo
            .crypto
            .keys()
            .ok_or_else(|| anyhow!("this backup is not encrypted"))?;
        // new key parameters: a pending change keeps its own
        let resumed = marker.is_some();
        let (marker, new_crypto) = match marker {
            Some(m) => {
                let c = derive_new(new, &m.kdf, old_keys)?;
                let ok = seal::unhex(&m.check)
                    .ok()
                    .and_then(|b| c.open_bytes("bookx-repo.json#check", &b, 1024).ok())
                    .is_some_and(|t| t == CHECK_TEXT);
                if !ok {
                    bail!("an earlier change to another new passphrase was not finished; use that new passphrase to finish it");
                }
                (m, c)
            }
            None => {
                let kc = KdfConfig {
                    algorithm: "argon2id".into(),
                    memory_kib: opts.kdf.memory_kib,
                    passes: opts.kdf.passes,
                    lanes: opts.kdf.lanes,
                    salt: seal::random_hex(16)?,
                };
                let c = derive_new(new, &kc, old_keys)?;
                let check = seal::hex(&c.seal_bytes("bookx-repo.json#check", CHECK_TEXT)?);
                let id_key = seal::hex(&c.seal_bytes("bookx-repo.json#idkey", &old_keys.id_key())?);
                let m = Marker {
                    format: FORMAT,
                    repo: repo.id.clone(),
                    id: seal::random_hex(16)?,
                    started: chrono::Utc::now().to_rfc3339(),
                    kdf: kc,
                    check,
                    id_key,
                };
                repo.store
                    .put_bytes(MARKER, &serde_json::to_vec_pretty(&m)?, cancel)?;
                (m, c)
            }
        };

        // every object of the repository
        let mut objects: Vec<(String, u64)> = Vec::new();
        for dir in ["snapshots", "journal"] {
            for e in repo.store.list(dir)? {
                if !e.is_dir && !e.name.starts_with('.') {
                    objects.push((format!("{dir}/{}", e.name), e.size));
                }
            }
        }
        if let Some(sz) = repo.store.stat("gc/pending.json")? {
            objects.push(("gc/pending.json".into(), sz));
        }
        for d in repo.store.list("blobs")? {
            if !d.is_dir || d.name.len() != 2 {
                continue;
            }
            for e in repo.store.list(&format!("blobs/{}", d.name))? {
                if !e.is_dir && seal::valid_id(&e.name) {
                    objects.push((format!("blobs/{}/{}", d.name, e.name), e.size));
                }
            }
        }
        let bytes_total: u64 = objects.iter().map(|o| o.1).sum();
        let done = AtomicUsize::new(0);
        let bytes_done = AtomicU64::new(0);
        let converted = AtomicUsize::new(0);
        let already = AtomicUsize::new(0);
        let failure: Mutex<Option<anyhow::Error>> = Mutex::new(None);
        let stop = AtomicBool::new(false);
        let emit = |current: &str| {
            progress(&RekeyProgress {
                objects_done: done.load(Ordering::Relaxed),
                objects_total: objects.len(),
                bytes_done: bytes_done.load(Ordering::Relaxed),
                bytes_total,
                current: current.to_string(),
            })
        };
        emit("");
        parallel_for(objects.len(), opts.jobs, |i| {
            if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                return;
            }
            let (path, stored) = &objects[i];
            let one = || -> Result<bool> {
                if opens_with(repo.store.as_ref(), &new_crypto, path, *stored) {
                    return Ok(false);
                }
                convert(
                    repo.store.as_ref(),
                    &repo.crypto,
                    &new_crypto,
                    path,
                    *stored,
                    cancel,
                )?;
                if opts.verify {
                    let n = repo.store.stat(path)?;
                    let want = seal::plain_len(*stored)
                        .map(|p| new_crypto.stored_len(p))
                        .unwrap_or(0);
                    if n != Some(want) {
                        bail!("{path} was not written completely");
                    }
                    let mut sink = std::io::sink();
                    let plain = seal::plain_len(*stored).unwrap_or(0);
                    let mut w = new_crypto.open_writer(path, &mut sink, plain);
                    repo.store.get_to(path, &mut w, want)?;
                    w.finish()
                        .with_context(|| format!("{path} does not read back correctly"))?;
                }
                Ok(true)
            };
            match one() {
                Ok(true) => {
                    converted.fetch_add(1, Ordering::Relaxed);
                }
                Ok(false) => {
                    already.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    if e.downcast_ref::<Cancelled>().is_none() {
                        stop.store(true, Ordering::Relaxed);
                        let mut f = failure.lock().unwrap();
                        if f.is_none() {
                            *f = Some(e);
                        }
                    }
                    return;
                }
            }
            done.fetch_add(1, Ordering::Relaxed);
            bytes_done.fetch_add(*stored, Ordering::Relaxed);
            emit(path);
        });
        if let Some(e) = failure.into_inner().unwrap() {
            return Err(e);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow::Error::new(Cancelled));
        }
        repo.lock_held()?;

        // the commit: new settings in one atomic write
        let new_cfg = RepoConfig {
            kdf: Some(marker.kdf.clone()),
            check: Some(marker.check.clone()),
            id_key: Some(marker.id_key.clone()),
            rekeyed: Some(marker.id.clone()),
            ..cfg
        };
        repo.store
            .put_bytes(CONFIG, &serde_json::to_vec_pretty(&new_cfg)?, cancel)?;
        let _ = repo.store.delete(MARKER);
        Ok(RekeyReport {
            objects: objects.len(),
            converted: converted.load(Ordering::Relaxed),
            already: already.load(Ordering::Relaxed),
            bytes: bytes_total,
            resumed,
            was_done: false,
        })
    }
}
