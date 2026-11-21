//! Bookx's own encrypted password store, for systems with no keyring.
//!
//! One file (`secrets.vault`, mode 0600) protected by a master password the user chooses. The
//! master password is never written anywhere; the key is derived with Argon2id and every item is
//! sealed with ChaCha20-Poly1305. Layout (all integers big endian):
//!
//! ```text
//! "BOOKXVLT" | version u8 | memory KiB u32 | passes u32 | lanes u32 | salt 16 | seq u64 | count u32   (header, 49 bytes)
//! count x ( name length u16 | name | nonce 12 | ciphertext length u32 | ciphertext + tag )
//! HMAC-SHA256 over everything before it (32 bytes)
//! ```
//!
//! Two keys come from the Argon2 output: one for the items, one for the MAC. An item's associated
//! data is the header plus its own name, so values cannot be swapped between names and an old
//! item cannot be pasted into a newer file. The MAC is checked first, in constant time, before any
//! item is touched: a wrong password and a damaged file give the same answer, and no partial data.
//! Key names are stored in clear (they are fixed words such as `email-password`); values are not.
//!
//! Limits: the file is at most [`MAX_FILE`] bytes, [`MAX_ITEMS`] items, names [`MAX_NAME`] bytes,
//! values [`MAX_VALUE`] bytes, and the Argon2 settings read from the file must be within
//! [`kdf_is_reasonable`] (a hostile file cannot ask for gigabytes).
//!
//! Rollback: `seq` goes up on every write. A write is refused when the file on disk no longer has
//! the sequence this session last saw, so a swapped-in older file or a second Bookx cannot be
//! silently overwritten. Putting an older copy back *between* sessions cannot be detected (there
//! is nowhere safe to keep a counter); it can only bring back older passwords of your own.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

use crate::SecretStore;

pub const MAGIC: &[u8; 8] = b"BOOKXVLT";
pub const VERSION: u8 = 1;
const HEADER: usize = 8 + 1 + 4 + 4 + 4 + 16 + 8 + 4;
pub const MAX_FILE: usize = 1 << 20;
pub const MAX_ITEMS: usize = 512;
pub const MAX_NAME: usize = 128;
pub const MAX_VALUE: usize = 16 * 1024;
/// Shortest master password Bookx accepts.
pub const MIN_PASSPHRASE: usize = 10;

pub const MAX_KDF_KIB: u32 = 256 * 1024;
pub const MAX_KDF_PASSES: u32 = 10;
pub const MAX_KDF_LANES: u32 = 8;

/// Are key derivation settings read from a file within what Bookx would ever write?
pub fn kdf_is_reasonable(memory_kib: u32, passes: u32, lanes: u32) -> bool {
    (8..=MAX_KDF_KIB).contains(&memory_kib)
        && (1..=MAX_KDF_PASSES).contains(&passes)
        && (1..=MAX_KDF_LANES).contains(&lanes)
}

/// Argon2id cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kdf {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

impl Kdf {
    /// 64 MiB, 3 passes, 1 lane: the password is asked once per session, so this can be heavier
    /// than the interactive minimum.
    pub const DEFAULT: Kdf = Kdf {
        memory_kib: 64 * 1024,
        passes: 3,
        lanes: 1,
    };
    /// For tests only.
    pub const TEST: Kdf = Kdf {
        memory_kib: 64,
        passes: 1,
        lanes: 1,
    };
}

/// Why a vault could not be opened. A wrong password and a damaged or tampered file are one case on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// There is no vault file.
    Missing,
    /// Wrong master password, or the file was damaged or changed.
    WrongOrDamaged,
    /// Not a Bookx vault, a newer version, or limits exceeded.
    Unreadable(String),
    Io(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Missing => write!(f, "there is no password store yet"),
            OpenError::WrongOrDamaged => {
                write!(f, "wrong master password, or the password store is damaged")
            }
            OpenError::Unreadable(s) => write!(f, "the password store cannot be read ({s})"),
            OpenError::Io(s) => write!(f, "the password store cannot be read ({s})"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Why a master password is refused (for the dialog; the texts are English source strings).
pub fn check_passphrase(p: &str) -> Result<(), &'static str> {
    if p.chars().count() < MIN_PASSPHRASE {
        return Err("Use at least 10 characters.");
    }
    let mut it = p.chars();
    let first = it.next().unwrap_or(' ');
    if it.all(|c| c == first) {
        return Err("Use more than one repeated character.");
    }
    Ok(())
}

type HmacSha256 = Hmac<Sha256>;

struct Open {
    enc: Zeroizing<[u8; 32]>,
    mac: Zeroizing<[u8; 32]>,
    salt: [u8; 16],
    kdf: Kdf,
    seq: u64,
    items: BTreeMap<String, Zeroizing<String>>,
}

/// The encrypted file. Locked until [`FileStore::unlock`] or [`FileStore::create`].
pub struct FileStore {
    path: PathBuf,
    kdf: Kdf,
    state: Mutex<Option<Open>>,
}

fn derive(pass: &str, salt: &[u8], kdf: Kdf) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(kdf.memory_kib, kdf.passes, kdf.lanes, Some(32))
        .map_err(|e| anyhow!("bad key derivation settings: {e}"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pass.as_bytes(), salt, &mut *key)
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

fn subkey(master: &[u8; 32], label: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut m = <HmacSha256 as Mac>::new_from_slice(master).expect("hmac takes any key length");
    m.update(label);
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&m.finalize().into_bytes());
    out
}

type Key32 = Zeroizing<[u8; 32]>;

fn keys(pass: &str, salt: &[u8], kdf: Kdf) -> Result<(Key32, Key32)> {
    let master = derive(pass, salt, kdf)?;
    Ok((
        subkey(&master, b"bookx-vault item key v1"),
        subkey(&master, b"bookx-vault mac key v1"),
    ))
}

fn random(buf: &mut [u8]) -> Result<()> {
    getrandom::getrandom(buf).map_err(|e| anyhow!("no random numbers available: {e}"))
}

fn header_bytes(o: &Open) -> Vec<u8> {
    let mut h = Vec::with_capacity(HEADER);
    h.extend_from_slice(MAGIC);
    h.push(VERSION);
    h.extend_from_slice(&o.kdf.memory_kib.to_be_bytes());
    h.extend_from_slice(&o.kdf.passes.to_be_bytes());
    h.extend_from_slice(&o.kdf.lanes.to_be_bytes());
    h.extend_from_slice(&o.salt);
    h.extend_from_slice(&o.seq.to_be_bytes());
    h.extend_from_slice(&(o.items.len() as u32).to_be_bytes());
    h
}

fn aad(header: &[u8], name: &str) -> Vec<u8> {
    let mut a = header.to_vec();
    a.extend_from_slice(name.as_bytes());
    a
}

fn seal(o: &Open) -> Result<Vec<u8>> {
    let header = header_bytes(o);
    let mut out = header.clone();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&*o.enc));
    for (name, value) in &o.items {
        let mut nonce = [0u8; 12];
        random(&mut nonce)?;
        let ct = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: value.as_bytes(),
                    aad: &aad(&header, name),
                },
            )
            .map_err(|_| anyhow!("encryption failed"))?;
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&(ct.len() as u32).to_be_bytes());
        out.extend_from_slice(&ct);
    }
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&*o.mac).expect("key length");
    mac.update(&out);
    out.extend_from_slice(&mac.finalize().into_bytes());
    if out.len() > MAX_FILE {
        bail!("too many passwords for the store");
    }
    Ok(out)
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| {
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            u64::from_be_bytes(a)
        })
    }
}

struct Head {
    kdf: Kdf,
    salt: [u8; 16],
    seq: u64,
    count: u32,
}

fn parse_header(data: &[u8]) -> Result<Head, OpenError> {
    let bad = |s: &str| OpenError::Unreadable(s.to_string());
    if data.len() > MAX_FILE {
        return Err(bad("the file is too large"));
    }
    if data.len() < HEADER + 32 || &data[..8] != MAGIC {
        return Err(bad("not a Bookx password store"));
    }
    let mut r = Reader(&data[8..HEADER]);
    let version = r.take(1).map(|b| b[0]).unwrap_or(0);
    if version != VERSION {
        return Err(bad("made by a different version of Bookx"));
    }
    let (m, p, l) = (r.u32().unwrap(), r.u32().unwrap(), r.u32().unwrap());
    if !kdf_is_reasonable(m, p, l) {
        return Err(bad("unreasonable key settings"));
    }
    let mut salt = [0u8; 16];
    salt.copy_from_slice(r.take(16).unwrap());
    let seq = r.u64().unwrap();
    let count = r.u32().unwrap();
    if count as usize > MAX_ITEMS {
        return Err(bad("too many items"));
    }
    Ok(Head {
        kdf: Kdf {
            memory_kib: m,
            passes: p,
            lanes: l,
        },
        salt,
        seq,
        count,
    })
}

fn open_bytes(data: &[u8], pass: &str) -> Result<Open, OpenError> {
    let h = parse_header(data)?;
    let (enc, mac_key) = keys(pass, &h.salt, h.kdf).map_err(|e| OpenError::Io(e.to_string()))?;
    let (body, tag) = data.split_at(data.len() - 32);
    let mut mac = <HmacSha256 as Mac>::new_from_slice(&*mac_key).expect("key length");
    mac.update(body);
    // constant time; nothing is decrypted or parsed before this passes
    if mac.verify_slice(tag).is_err() {
        return Err(OpenError::WrongOrDamaged);
    }
    let mut o = Open {
        enc,
        mac: mac_key,
        salt: h.salt,
        kdf: h.kdf,
        seq: h.seq,
        items: BTreeMap::new(),
    };
    let header = &body[..HEADER];
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&*o.enc));
    let mut r = Reader(&body[HEADER..]);
    for _ in 0..h.count {
        let item = (|| {
            let nl = r.u16()? as usize;
            if nl == 0 || nl > MAX_NAME {
                return None;
            }
            let name = std::str::from_utf8(r.take(nl)?).ok()?.to_string();
            let nonce = r.take(12)?;
            let cl = r.u32()? as usize;
            if !(16..=MAX_VALUE + 16).contains(&cl) {
                return None;
            }
            let ct = r.take(cl)?;
            let plain = cipher
                .decrypt(
                    Nonce::from_slice(nonce),
                    Payload {
                        msg: ct,
                        aad: &aad(header, &name),
                    },
                )
                .ok()?;
            let plain = Zeroizing::new(plain);
            let value = String::from_utf8(plain.to_vec()).ok()?;
            Some((name, value))
        })();
        // authenticated but malformed: our own bug or a future format; same answer as tampering
        let Some((name, value)) = item else {
            return Err(OpenError::WrongOrDamaged);
        };
        o.items.insert(name, Zeroizing::new(value));
    }
    if !r.0.is_empty() {
        return Err(OpenError::WrongOrDamaged);
    }
    Ok(o)
}

/// Opens vault bytes without touching the disk and returns the number of items: for the fuzz target
/// and the hostile-input tests. Never panics, never allocates beyond [`MAX_FILE`].
pub fn probe(data: &[u8], passphrase: &str) -> Result<usize, OpenError> {
    open_bytes(data, passphrase).map(|o| o.items.len())
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    restrict_dir(dir);
    let tmp = path.with_extension("vault.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut f = opts.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Makes the settings folder private to the user (best effort, Unix only).
pub fn restrict_dir(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Makes one file readable by the user only (best effort, Unix only).
pub fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn disk_seq(path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = [0u8; HEADER];
    f.read_exact(&mut h).ok()?;
    if &h[..8] != MAGIC {
        return None;
    }
    let mut a = [0u8; 8];
    a.copy_from_slice(&h[37..45]);
    Some(u64::from_be_bytes(a))
}

impl FileStore {
    pub fn new(path: impl Into<PathBuf>) -> FileStore {
        FileStore {
            path: path.into(),
            kdf: Kdf::DEFAULT,
            state: Mutex::new(None),
        }
    }

    /// Same, with another key derivation cost (tests).
    pub fn with_kdf(path: impl Into<PathBuf>, kdf: Kdf) -> FileStore {
        FileStore {
            kdf,
            ..FileStore::new(path)
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    pub fn is_unlocked(&self) -> bool {
        self.state.lock().unwrap().is_some()
    }

    /// Makes a new, empty store protected by `passphrase`. Never replaces an existing file.
    pub fn create(&self, passphrase: &str) -> Result<()> {
        check_passphrase(passphrase).map_err(|e| anyhow!("{e}"))?;
        if self.exists() {
            bail!("a password store already exists");
        }
        let mut salt = [0u8; 16];
        random(&mut salt)?;
        let (enc, mac) = keys(passphrase, &salt, self.kdf)?;
        let o = Open {
            enc,
            mac,
            salt,
            kdf: self.kdf,
            seq: 1,
            items: BTreeMap::new(),
        };
        write_atomic(&self.path, &seal(&o)?)?;
        *self.state.lock().unwrap() = Some(o);
        Ok(())
    }

    pub fn unlock(&self, passphrase: &str) -> Result<(), OpenError> {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(OpenError::Missing),
            Err(e) => return Err(OpenError::Io(e.kind().to_string())),
        };
        let o = open_bytes(&data, passphrase)?;
        *self.state.lock().unwrap() = Some(o);
        Ok(())
    }

    /// Forgets the keys and values held in memory.
    pub fn lock(&self) {
        *self.state.lock().unwrap() = None;
    }

    /// Re-encrypts the store under a new master password (new salt).
    pub fn change_passphrase(&self, new: &str) -> Result<()> {
        check_passphrase(new).map_err(|e| anyhow!("{e}"))?;
        let mut g = self.state.lock().unwrap();
        let Some(o) = g.as_mut() else {
            bail!("the password store is locked");
        };
        self.check_disk(o)?;
        let mut salt = [0u8; 16];
        random(&mut salt)?;
        let (enc, mac) = keys(new, &salt, self.kdf)?;
        let mut next = Open {
            enc,
            mac,
            salt,
            kdf: self.kdf,
            seq: o.seq + 1,
            items: o.items.clone(),
        };
        write_atomic(&self.path, &seal(&next)?)?;
        std::mem::swap(o, &mut next);
        Ok(())
    }

    /// Deletes the file and forgets everything (the user lost the master password).
    pub fn destroy(&self) -> Result<()> {
        *self.state.lock().unwrap() = None;
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn keys(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .as_ref()
            .map(|o| o.items.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn check_disk(&self, o: &Open) -> Result<()> {
        match disk_seq(&self.path) {
            Some(s) if s == o.seq => Ok(()),
            _ => bail!("the password store was changed by something else; restart Bookx"),
        }
    }

    fn commit(&self, f: impl FnOnce(&mut BTreeMap<String, Zeroizing<String>>)) -> Result<()> {
        let mut g = self.state.lock().unwrap();
        let Some(o) = g.as_mut() else {
            bail!("the password store is locked");
        };
        self.check_disk(o)?;
        let before = o.items.clone();
        f(&mut o.items);
        if o.items.len() > MAX_ITEMS {
            o.items = before;
            bail!("too many passwords for the store");
        }
        o.seq += 1;
        match seal(o).and_then(|d| write_atomic(&self.path, &d)) {
            Ok(()) => Ok(()),
            Err(e) => {
                o.items = before;
                o.seq -= 1;
                Err(e)
            }
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        self.salt.zeroize();
    }
}

impl SecretStore for FileStore {
    fn describe(&self) -> String {
        "Bookx's encrypted password store".into()
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        match self.state.lock().unwrap().as_ref() {
            None => bail!("the password store is locked"),
            Some(o) => Ok(o.items.get(key).map(|v| v.to_string())),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        if key.is_empty() || key.len() > MAX_NAME || value.len() > MAX_VALUE {
            bail!("the value does not fit the password store");
        }
        if self
            .state
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|o| o.items.get(key).is_some_and(|v| v.as_str() == value))
        {
            return Ok(());
        }
        let (k, v) = (key.to_string(), Zeroizing::new(value.to_string()));
        self.commit(move |m| {
            m.insert(k, v);
        })
    }

    fn delete(&self, key: &str) -> Result<()> {
        if !self
            .state
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|o| o.items.contains_key(key))
        {
            return Ok(());
        }
        let k = key.to_string();
        self.commit(move |m| {
            m.remove(&k);
        })
    }

    fn available(&self) -> bool {
        self.is_unlocked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-vault-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn store(d: &Path) -> FileStore {
        FileStore::with_kdf(d.join("secrets.vault"), Kdf::TEST)
    }

    const PASS: &str = "correct horse battery";

    #[test]
    fn values_round_trip_through_the_file_and_a_new_session() {
        let d = dir("rt");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.set("email-password", "sentinel-AAAA-1").unwrap();
        s.set("tts-api-key", "sentinel-BBBB-2").unwrap();
        s.delete("tts-api-key").unwrap();
        let raw = std::fs::read(s.path()).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("sentinel"));
        let again = store(&d);
        assert!(
            again.get("email-password").is_err(),
            "locked until unlocked"
        );
        again.unlock(PASS).unwrap();
        assert_eq!(
            again.get("email-password").unwrap().as_deref(),
            Some("sentinel-AAAA-1")
        );
        assert_eq!(again.get("tts-api-key").unwrap(), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_wrong_passphrase_and_a_damaged_file_give_the_same_answer() {
        let d = dir("wrong");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.set("a", "value-1").unwrap();
        assert_eq!(
            store(&d).unlock("not the passphrase").unwrap_err(),
            OpenError::WrongOrDamaged
        );
        let mut raw = std::fs::read(s.path()).unwrap();
        let n = raw.len();
        raw[n / 2] ^= 1;
        std::fs::write(s.path(), &raw).unwrap();
        assert_eq!(
            store(&d).unlock(PASS).unwrap_err(),
            OpenError::WrongOrDamaged
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_single_byte_flip_and_every_truncation_is_refused() {
        let d = dir("tamper");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.set("a", "value-1").unwrap();
        s.set("b", "value-2").unwrap();
        let raw = std::fs::read(s.path()).unwrap();
        for i in 0..raw.len() {
            let mut x = raw.clone();
            x[i] ^= 0x80;
            assert!(open_bytes(&x, PASS).is_err(), "flip at {i}");
        }
        for n in 0..raw.len() {
            assert!(open_bytes(&raw[..n], PASS).is_err(), "cut at {n}");
        }
        let mut extra = raw.clone();
        extra.push(0);
        assert!(open_bytes(&extra, PASS).is_err());
        assert!(open_bytes(&raw, PASS).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_hostile_file_cannot_ask_for_gigabytes_or_a_huge_item_count() {
        let d = dir("evil");
        let s = store(&d);
        s.create(PASS).unwrap();
        let raw = std::fs::read(s.path()).unwrap();
        let mut big = raw.clone();
        big[9..13].copy_from_slice(&(4u32 << 20).to_be_bytes()); // 4 GiB of memory
        assert!(matches!(
            open_bytes(&big, PASS),
            Err(OpenError::Unreadable(_))
        ));
        let mut many = raw.clone();
        many[45..49].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            open_bytes(&many, PASS),
            Err(OpenError::Unreadable(_))
        ));
        assert!(open_bytes(&vec![0u8; MAX_FILE + 1], PASS).is_err());
        assert!(open_bytes(b"", PASS).is_err());
        assert!(open_bytes(b"BOOKXVLT", PASS).is_err());
        assert!(!kdf_is_reasonable(u32::MAX, 3, 1));
        assert!(!kdf_is_reasonable(64, 0, 1));
        assert!(!kdf_is_reasonable(64, 3, 99));
        assert!(kdf_is_reasonable(Kdf::DEFAULT.memory_kib, 3, 1));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_older_file_put_back_during_a_session_is_not_overwritten() {
        let d = dir("rollback");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.set("a", "v1").unwrap();
        let old = std::fs::read(s.path()).unwrap();
        s.set("a", "v2").unwrap();
        std::fs::write(s.path(), old).unwrap();
        assert!(s.set("a", "v3").is_err(), "the sequence on disk went back");
        assert!(s.delete("a").is_err());
        // the damaged-looking file is untouched
        assert!(store(&d).unlock(PASS).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_item_cannot_be_moved_to_another_name() {
        let d = dir("swap");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.set("aaaa", "value-1").unwrap();
        let raw = std::fs::read(s.path()).unwrap();
        // rename "aaaa" to "bbbb" and fix up the MAC with the real key: the item AEAD still refuses
        let mut x = raw.clone();
        let at = raw.windows(4).position(|w| w == b"aaaa").unwrap();
        x[at..at + 4].copy_from_slice(b"bbbb");
        let h = parse_header(&x).unwrap();
        let (_, mac_key) = keys(PASS, &h.salt, h.kdf).unwrap();
        let body_len = x.len() - 32;
        let mut m = <HmacSha256 as Mac>::new_from_slice(&*mac_key).unwrap();
        m.update(&x[..body_len]);
        x[body_len..].copy_from_slice(&m.finalize().into_bytes());
        assert_eq!(open_bytes(&x, PASS).err(), Some(OpenError::WrongOrDamaged));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_the_folder_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("mode");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.set("a", "v").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(s.path()), 0o600);
        assert_eq!(mode(&d), 0o700);
        assert!(!d.join("secrets.vault.tmp").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn create_never_overwrites_and_weak_passphrases_are_refused() {
        let d = dir("create");
        let s = store(&d);
        assert!(s.create("short").is_err());
        assert!(s.create("aaaaaaaaaaaaaa").is_err());
        assert!(!s.exists());
        s.create(PASS).unwrap();
        assert!(store(&d).create("another passphrase").is_err());
        assert_eq!(
            store(&d).unlock("x").unwrap_err(),
            OpenError::WrongOrDamaged
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_passphrase_can_be_changed_and_the_store_can_be_locked() {
        let d = dir("change");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.set("a", "v1").unwrap();
        s.change_passphrase("a brand new passphrase").unwrap();
        s.lock();
        assert!(s.set("a", "v2").is_err());
        assert!(store(&d).unlock(PASS).is_err());
        let t = store(&d);
        t.unlock("a brand new passphrase").unwrap();
        assert_eq!(t.get("a").unwrap().as_deref(), Some("v1"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn destroying_the_store_deletes_the_file_and_forgets_the_keys() {
        let d = dir("destroy");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.set("b", "2").unwrap();
        s.set("a", "1").unwrap();
        assert_eq!(s.keys(), ["a", "b"]);
        s.destroy().unwrap();
        assert!(!s.exists());
        assert!(!s.is_unlocked());
        assert!(s.keys().is_empty());
        // destroying what is not there is not an error, and a new store can follow
        s.destroy().unwrap();
        s.create(PASS).unwrap();
        assert!(s.keys().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_locked_store_refuses_to_change_its_passphrase_or_a_value() {
        let d = dir("locked-change");
        let s = store(&d);
        s.create(PASS).unwrap();
        s.lock();
        let e = s.change_passphrase("a brand new passphrase").unwrap_err();
        assert!(e.to_string().contains("locked"));
        assert!(s.get("a").unwrap_err().to_string().contains("locked"));
        // a weak new passphrase is refused before anything is touched
        s.unlock(PASS).unwrap();
        assert!(s.change_passphrase("short").is_err());
        assert!(store(&d).unlock(PASS).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_errors_say_what_is_wrong_without_the_secret() {
        assert_eq!(
            OpenError::Missing.to_string(),
            "there is no password store yet"
        );
        assert!(OpenError::WrongOrDamaged
            .to_string()
            .contains("wrong master password"));
        assert_eq!(
            OpenError::Unreadable("version 9".into()).to_string(),
            "the password store cannot be read (version 9)"
        );
        assert_eq!(
            OpenError::Io("permission denied".into()).to_string(),
            "the password store cannot be read (permission denied)"
        );
    }

    #[test]
    fn missing_file_and_limits() {
        let d = dir("missing");
        let s = store(&d);
        assert_eq!(s.unlock(PASS).unwrap_err(), OpenError::Missing);
        s.create(PASS).unwrap();
        assert!(s.set("k", &"x".repeat(MAX_VALUE + 1)).is_err());
        assert!(s.set(&"k".repeat(MAX_NAME + 1), "v").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
