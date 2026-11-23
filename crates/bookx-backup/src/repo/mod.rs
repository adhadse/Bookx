//! The incremental backup repository (decision D9 in `docs/backup-notes.md`).
//!
//! A repository is a folder named [`REPO_DIR`] inside a place (folder, drive, share, cloud-drive
//! mount, WebDAV). It holds the book files, covers and catalogue of a library, each stored once:
//!
//! ```text
//! Bookx Backup/
//!   README.txt              what this is and how to get files out by hand
//!   bookx-repo.json         format, repository id, encryption parameters (readable)
//!   blobs/ab/<id>           file contents, named by their hash (plain copies when not encrypted)
//!   snapshots/s…-<time>-<kind>.snap   manifests: path -> blob, plus the catalogue and notes blobs
//!   journal/j…-<time>.jrn   small records of changed highlights/notes/bookmarks/progress
//!   gc/pending.json         blobs no snapshot uses, waiting out their grace period
//! ```
//!
//! * A run copies only blobs the place does not have yet; renamed or moved files cost nothing.
//! * The snapshot manifest is written last, so a crashed or cancelled run leaves only unused blobs.
//! * Restoring takes a snapshot (or the newest), writes files and the catalogue, then replays the
//!   journal up to a point in time.
//!
//! See [`Repository`] for the API and [`seal`] for the encryption.

pub mod lock;
pub mod maint;
pub mod manifest;
pub mod rekey;
pub mod restore;
pub mod run;
pub mod seal;
pub mod source;
pub mod store;

use std::io::Write;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::{self, Kdf};
use manifest::{
    MAX_CATALOGUE_BYTES, MAX_JOURNAL_BYTES, MAX_MANIFEST_BYTES, MAX_STATE_BYTES, Manifest, SnapName,
};
use seal::{Crypto, IdHasher, Keys, ids_equal};
use store::RepoStore;

pub use lock::{LockInfo, LockLost, RepoLocked};
pub use maint::{GcReport, VerifyReport};
pub use rekey::{RekeyOptions, RekeyPending, RekeyProgress, RekeyReport};
pub use restore::{CatalogueAction, ReplayReport, RestoreMode, RestoreOptions, RestoreReport};
pub use run::{JournalAppend, Phase, Plan, Report, RunOptions, RunProgress, estimate_new};
pub use source::{LibrarySource, Scope};
pub use store::{DavStore, FolderStore, MemStore, REPO_DIR};

const CONFIG: &str = "bookx-repo.json";
const README: &str = "README.txt";
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// What the repository needs to be opened. The passphrase is never printed.
#[derive(Clone, Default)]
pub struct Secrets {
    pub passphrase: Option<String>,
}

impl Secrets {
    pub fn none() -> Secrets {
        Secrets { passphrase: None }
    }

    pub fn passphrase(p: &str) -> Secrets {
        Secrets {
            passphrase: (!p.is_empty()).then(|| p.to_string()),
        }
    }
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Secrets {{ passphrase: {} }}",
            if self.passphrase.is_some() {
                "<set>"
            } else {
                "<none>"
            }
        )
    }
}

/// The place has no repository (yet).
#[derive(Debug)]
pub struct NoRepository;

impl std::fmt::Display for NoRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("there is no Bookx backup repository in this place")
    }
}
impl std::error::Error for NoRepository {}

/// A run that found the place older than what this library saw before.
#[derive(Debug)]
pub struct RolledBack {
    pub seen: u64,
    pub found: u64,
}

impl std::fmt::Display for RolledBack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the backup place is older than before (newest snapshot {} here, {} seen earlier): a copy may have been replaced by an old one or snapshots deleted. Check the place, then confirm to continue",
            self.found, self.seen
        )
    }
}
impl std::error::Error for RolledBack {}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KdfConfig {
    algorithm: String,
    memory_kib: u32,
    passes: u32,
    lanes: u32,
    salt: String,
}

/// `bookx-repo.json`: readable by anyone, secret by nobody.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RepoConfig {
    format: u32,
    repo: String,
    created: String,
    app: String,
    encrypted: bool,
    #[serde(default)]
    kdf: Option<KdfConfig>,
    /// A sealed known text: opening it proves the passphrase.
    #[serde(default)]
    check: Option<String>,
    /// After a passphrase change: the id key of the first passphrase, sealed under the current
    /// keys (object names were made with it and stay as they are).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id_key: Option<String>,
    /// After a passphrase change: the id of the change that produced this file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rekeyed: Option<String>,
}

const CHECK_TEXT: &[u8] = b"bookx-repository";

/// An opened repository. All methods take `&self`; several threads may work on one.
pub struct Repository {
    pub(crate) store: Box<dyn RepoStore>,
    pub(crate) id: String,
    pub(crate) crypto: Crypto,
    pub(crate) lock_state: std::sync::Mutex<lock::LockState>,
}

/// One snapshot in the place, from its file name alone (nothing is decrypted to list them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub name: SnapName,
    pub size: u64,
}

impl SnapshotInfo {
    pub fn file_name(&self) -> String {
        self.name.file_name()
    }
}

pub(crate) fn blob_path(id: &str) -> String {
    format!("blobs/{}/{}", &id[..2], id)
}

fn readme(encrypted: bool) -> String {
    let mut t = String::from(
        "This folder is a Bookx backup repository.\n\n\
         It is written and read by Bookx (the app or `bookx-cli backup`). Do not edit it by hand.\n\n\
         blobs/       the files of your library, each stored once under the hash of its content\n\
         snapshots/   one manifest per backup: which path in your library is which blob\n\
         journal/     small records of changed highlights, notes, bookmarks and reading progress\n\
         bookx-repo.json  format and settings (no secrets)\n\n",
    );
    if encrypted {
        t.push_str(
            "This repository is ENCRYPTED. Without the passphrase nothing in blobs/, snapshots/\n\
             and journal/ can be read, by anybody. Keep the passphrase safe: it cannot be recovered.\n\
             Restore with Bookx (Library Health > Backups) or `bookx-cli backup restore`.\n",
        );
    } else {
        t.push_str(
            "This repository is NOT encrypted. Every file under blobs/ is an exact copy of a file\n\
             of your library (an e-book, a cover, the catalogue database); its name is the SHA-256\n\
             of its content. To get a book out without Bookx, open the newest file in snapshots/\n\
             (plain JSON): the \"files\" list gives each path and the blob that holds it, so\n\
             blobs/ab/abcdef... can be copied and renamed to its path.\n\
             Whoever can read this place can read your books, notes and catalogue.\n",
        );
    }
    t
}

/// The keys of an encrypted repository for a passphrase; `None` when the passphrase is wrong.
fn crypto_for(cfg: &RepoConfig, secrets: &Secrets) -> Result<Option<Crypto>> {
    let k = cfg
        .kdf
        .as_ref()
        .filter(|k| k.algorithm == "argon2id")
        .ok_or_else(|| anyhow!("the repository uses an unknown key method"))?;
    let pass = secrets
        .passphrase
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            anyhow!("this backup is encrypted: enter the backup passphrase to open it")
        })?;
    if !crate::crypto::kdf_is_reasonable(k.memory_kib, k.passes, k.lanes) {
        bail!("the backup asks for unreasonable key settings; refusing to open it");
    }
    let salt = seal::unhex(&k.salt)?;
    if salt.len() < 16 || salt.len() > 64 {
        bail!("the repository settings (bookx-repo.json) are damaged");
    }
    let mut master = crypto::derive(
        pass,
        &salt,
        Kdf {
            memory_kib: k.memory_kib,
            passes: k.passes,
            lanes: k.lanes,
        },
    )?;
    let keys = Keys::from_master(&master);
    zeroize::Zeroize::zeroize(&mut master);
    let check = cfg
        .check
        .as_deref()
        .ok_or_else(|| anyhow!("the repository settings (bookx-repo.json) are damaged"))?;
    let sealed = seal::unhex(check)?;
    let c = Crypto::Keyed(keys);
    match c.open_bytes("bookx-repo.json#check", &sealed, 1024) {
        Ok(t) if ids_equal(&seal::hex(&t), &seal::hex(CHECK_TEXT)) => {}
        _ => return Ok(None),
    }
    Ok(Some(match &cfg.id_key {
        None => c,
        Some(sealed_id) => {
            let raw = c
                .open_bytes("bookx-repo.json#idkey", &seal::unhex(sealed_id)?, 64)
                .context("the repository settings (bookx-repo.json) are damaged")?;
            let idk: [u8; 32] = raw
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("the repository settings (bookx-repo.json) are damaged"))?;
            let Crypto::Keyed(mut k) = c else {
                unreachable!("keys were just made")
            };
            k.set_id_key(idk);
            Crypto::Keyed(k)
        }
    }))
}

impl Repository {
    /// Opens the repository in `store`. Fails with [`NoRepository`] (downcast the error) when the
    /// place has none, and with a clear message when it is encrypted and the passphrase is
    /// missing or wrong.
    pub fn open(store: Box<dyn RepoStore>, secrets: &Secrets) -> Result<Repository> {
        Repository::open_with(store, secrets, false)
    }

    pub(crate) fn open_with(
        store: Box<dyn RepoStore>,
        secrets: &Secrets,
        allow_unfinished_change: bool,
    ) -> Result<Repository> {
        let bytes = match store.stat(CONFIG)? {
            Some(_) => store
                .get_bytes(CONFIG, MAX_CONFIG_BYTES)
                .context("reading the repository settings")?,
            None => return Err(anyhow::Error::new(NoRepository)),
        };
        let cfg: RepoConfig = serde_json::from_slice(&bytes)
            .context("the repository settings (bookx-repo.json) are damaged")?;
        if cfg.format != manifest::FORMAT {
            bail!(
                "this repository has format {}, which this Bookx cannot read; update Bookx",
                cfg.format
            );
        }
        if cfg.repo.len() != 32 || !cfg.repo.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("the repository settings (bookx-repo.json) are damaged");
        }
        let crypto = if cfg.encrypted {
            match crypto_for(&cfg, secrets)? {
                Some(c) => c,
                None => {
                    // half way through a passphrase change the settings still describe the old one
                    if !allow_unfinished_change {
                        rekey::refuse_unfinished(store.as_ref(), &cfg)?;
                    }
                    bail!("wrong passphrase for this backup")
                }
            }
        } else {
            Crypto::Plain
        };
        if !allow_unfinished_change {
            rekey::refuse_unfinished(store.as_ref(), &cfg)?;
        }
        Ok(Repository {
            store,
            id: cfg.repo,
            crypto,
            lock_state: Default::default(),
        })
    }

    /// Creates a repository in an empty place. `encrypt` needs a passphrase in `secrets`.
    pub fn create(
        store: Box<dyn RepoStore>,
        secrets: &Secrets,
        encrypt: bool,
        kdf: Kdf,
    ) -> Result<Repository> {
        store.check()?;
        if store.stat(CONFIG)?.is_some() {
            bail!("there is a backup repository in this place already");
        }
        let id = seal::random_hex(16)?;
        let (crypto, kdf_cfg, check) = if encrypt {
            let pass = secrets
                .passphrase
                .as_deref()
                .filter(|p| !p.is_empty())
                .ok_or_else(|| anyhow!("set a backup passphrase to encrypt this destination"))?;
            let salt_hex = seal::random_hex(16)?;
            let mut master = crypto::derive(pass, &seal::unhex(&salt_hex)?, kdf)?;
            let c = Crypto::Keyed(Keys::from_master(&master));
            zeroize::Zeroize::zeroize(&mut master);
            let sealed = c.seal_bytes("bookx-repo.json#check", CHECK_TEXT)?;
            (
                c,
                Some(KdfConfig {
                    algorithm: "argon2id".into(),
                    memory_kib: kdf.memory_kib,
                    passes: kdf.passes,
                    lanes: kdf.lanes,
                    salt: salt_hex,
                }),
                Some(seal::hex(&sealed)),
            )
        } else {
            (Crypto::Plain, None, None)
        };
        let cfg = RepoConfig {
            format: manifest::FORMAT,
            repo: id.clone(),
            created: Utc::now().to_rfc3339(),
            app: format!("Bookx {}", env!("CARGO_PKG_VERSION")),
            encrypted: encrypt,
            kdf: kdf_cfg,
            check,
            id_key: None,
            rekeyed: None,
        };
        let no = AtomicBool::new(false);
        // the README first (harmless alone), the config last: a repository exists once it has one
        store.put_bytes(README, readme(encrypt).as_bytes(), &no)?;
        store.put_bytes(CONFIG, &serde_json::to_vec_pretty(&cfg)?, &no)?;
        Ok(Repository {
            store,
            id,
            crypto,
            lock_state: Default::default(),
        })
    }

    /// Opens the repository, or creates it when the place has none. An existing repository whose
    /// encryption differs from `encrypt` is an error (a different folder is needed).
    pub fn open_or_create(
        store: Box<dyn RepoStore>,
        secrets: &Secrets,
        encrypt: bool,
        kdf: Kdf,
    ) -> Result<Repository> {
        if store.stat(CONFIG)?.is_none() {
            return Repository::create(store, secrets, encrypt, kdf);
        }
        let r = Repository::open(store, secrets)?;
        if r.is_encrypted() != encrypt {
            bail!(
                "the backup place already has {} repository; use another folder or switch encryption {} for this destination",
                if r.is_encrypted() {
                    "an encrypted"
                } else {
                    "an unencrypted"
                },
                if r.is_encrypted() { "on" } else { "off" }
            );
        }
        Ok(r)
    }

    /// Whether the place has a repository (no passphrase needed).
    pub fn exists(store: &dyn RepoStore) -> Result<bool> {
        Ok(store.stat(CONFIG)?.is_some())
    }

    /// Whether the repository in the place is encrypted (readable without the passphrase).
    pub fn is_encrypted_at(store: &dyn RepoStore) -> Result<Option<bool>> {
        if store.stat(CONFIG)?.is_none() {
            return Ok(None);
        }
        let b = store.get_bytes(CONFIG, MAX_CONFIG_BYTES)?;
        let c: RepoConfig = serde_json::from_slice(&b)?;
        Ok(Some(c.encrypted))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn is_encrypted(&self) -> bool {
        self.crypto.is_encrypted()
    }

    pub fn label(&self) -> String {
        self.store.label()
    }

    pub fn store(&self) -> &dyn RepoStore {
        self.store.as_ref()
    }

    // -- snapshots ---------------------------------------------------------------------------------------

    /// The snapshots in the place, newest first.
    pub fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>> {
        let mut v: Vec<SnapshotInfo> = self
            .store
            .list("snapshots")?
            .into_iter()
            .filter(|e| !e.is_dir)
            .filter_map(|e| {
                Some(SnapshotInfo {
                    name: SnapName::parse(&e.name)?,
                    size: e.size,
                })
            })
            .collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.name.seq));
        Ok(v)
    }

    /// Reads and checks one snapshot's manifest.
    pub fn manifest(&self, name: &SnapName) -> Result<Manifest> {
        let path = name.path();
        let bytes = self
            .store
            .get_bytes(&path, self.crypto.stored_len(MAX_MANIFEST_BYTES))
            .with_context(|| format!("reading snapshot {}", name.file_name()))?;
        let plain = self
            .crypto
            .open_bytes(&path, &bytes, MAX_MANIFEST_BYTES)
            .with_context(|| format!("opening snapshot {}", name.file_name()))?;
        let m: Manifest = serde_json::from_slice(&plain)
            .with_context(|| format!("snapshot {} is damaged", name.file_name()))?;
        m.validate(&self.id, name)
            .with_context(|| format!("snapshot {} is not valid", name.file_name()))?;
        Ok(m)
    }

    /// The snapshot called `name` (a file name, with or without `.snap`, or its number), or the
    /// newest one at or before `until`.
    pub fn find_snapshot(
        &self,
        name: Option<&str>,
        until: Option<DateTime<Utc>>,
    ) -> Result<SnapshotInfo> {
        let all = self.list_snapshots()?;
        match name {
            Some(n) if n != "latest" => {
                let n = n.strip_suffix(".snap").unwrap_or(n);
                all.into_iter()
                    .find(|s| {
                        let f = s.name.file_name();
                        f.strip_suffix(".snap") == Some(n)
                            || n.parse::<u64>().ok() == Some(s.name.seq)
                    })
                    .ok_or_else(|| anyhow!("there is no snapshot {n:?} in this place"))
            }
            _ => all
                .into_iter()
                .find(|s| until.is_none_or(|u| s.name.at <= u))
                .ok_or_else(|| anyhow!("there are no snapshots in this place yet")),
        }
    }

    // -- blobs -------------------------------------------------------------------------------------------

    /// Streams a blob's content into `out`, checking it against its id (and `size`) on the way.
    /// On error `out` may hold a partial content: write to a temporary file.
    pub(crate) fn read_blob(&self, id: &str, size: u64, out: &mut dyn Write) -> Result<()> {
        if !seal::valid_id(id) {
            bail!("a bad object name in the snapshot");
        }
        let path = blob_path(id);
        let mut hw = HashWrite {
            inner: out,
            hasher: Some(self.crypto.id_hasher()),
            n: 0,
        };
        {
            let mut ow = self.crypto.open_writer(&path, &mut hw, size);
            self.store
                .get_to(&path, &mut ow, self.crypto.stored_len(size))
                .with_context(|| format!("reading {}", short(id)))?;
            ow.finish()
                .with_context(|| format!("opening {}", short(id)))?;
        }
        if hw.n != size {
            bail!(
                "{} is shorter than the snapshot says (truncated)",
                short(id)
            );
        }
        let got = hw.hasher.take().expect("hasher present").finish_hex();
        if !ids_equal(&got, id) {
            bail!(
                "{} does not match its checksum (damaged or tampered with)",
                short(id)
            );
        }
        Ok(())
    }

    pub(crate) fn read_blob_bytes(&self, id: &str, size: u64, max: u64) -> Result<Vec<u8>> {
        if size > max {
            bail!("an object in the snapshot is bigger than allowed");
        }
        let mut v = Vec::new();
        self.read_blob(id, size, &mut v)?;
        Ok(v)
    }

    /// The catalogue database bytes of a snapshot are written to `out`.
    pub fn read_catalogue(&self, m: &Manifest, out: &mut dyn Write) -> Result<()> {
        if m.catalogue.size > MAX_CATALOGUE_BYTES {
            bail!("the catalogue in the snapshot is bigger than allowed");
        }
        self.read_blob(&m.catalogue.blob, m.catalogue.size, out)
    }

    /// The notes state of a snapshot.
    pub fn read_state(&self, m: &Manifest) -> Result<manifest::LibraryState> {
        let b = self.read_blob_bytes(&m.state.blob, m.state.size, MAX_STATE_BYTES)?;
        serde_json::from_slice(&b).context("the notes in the snapshot are damaged")
    }

    // -- the journal -------------------------------------------------------------------------------------

    /// Journal records in the place, oldest first.
    pub fn list_journal(&self) -> Result<Vec<(u64, DateTime<Utc>, u64)>> {
        let mut v: Vec<_> = self
            .store
            .list("journal")?
            .into_iter()
            .filter(|e| !e.is_dir)
            .filter_map(|e| {
                let (seq, at) = manifest::parse_journal_name(&e.name)?;
                Some((seq, at, e.size))
            })
            .collect();
        v.sort_by_key(|x| x.0);
        Ok(v)
    }

    pub(crate) fn read_journal(
        &self,
        seq: u64,
        at: DateTime<Utc>,
    ) -> Result<manifest::JournalRecord> {
        let path = format!("journal/{}", manifest::journal_name(seq, at));
        let bytes = self
            .store
            .get_bytes(&path, self.crypto.stored_len(MAX_JOURNAL_BYTES))?;
        let plain = self.crypto.open_bytes(&path, &bytes, MAX_JOURNAL_BYTES)?;
        let r: manifest::JournalRecord = serde_json::from_slice(&plain)
            .with_context(|| format!("journal record {seq} is damaged"))?;
        if r.format != manifest::FORMAT || r.repo != self.id || r.seq != seq {
            bail!("journal record {seq} does not belong here");
        }
        Ok(r)
    }

    /// Writes a small object and reads it back to check it.
    pub(crate) fn put_checked(&self, path: &str, plain: &[u8], cancel: &AtomicBool) -> Result<()> {
        let sealed = self.crypto.seal_bytes(path, plain)?;
        self.store.put_bytes(path, &sealed, cancel)?;
        let back = self
            .store
            .get_bytes(path, sealed.len() as u64)
            .map_err(|e| {
                let _ = self.store.delete(path);
                anyhow!("could not read the copy back: {e:#}")
            })?;
        if Sha256::digest(&back) != Sha256::digest(&sealed) {
            let _ = self.store.delete(path);
            bail!(
                "the copy does not match the original (it was removed); the place may be full or unreliable"
            );
        }
        Ok(())
    }
}

pub(crate) fn short(id: &str) -> String {
    format!("object {}", &id[..id.len().min(12)])
}

pub(crate) struct HashWrite<'a> {
    pub inner: &'a mut dyn Write,
    pub hasher: Option<IdHasher>,
    pub n: u64,
}

impl Write for HashWrite<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.inner.write_all(b)?;
        if let Some(h) = &mut self.hasher {
            h.update(b);
        }
        self.n += b.len() as u64;
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
