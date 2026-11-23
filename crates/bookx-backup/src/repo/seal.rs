//! Encrypting the objects of a repository (blobs, snapshots, journal records) and the keyed blob
//! ids that let an encrypted place deduplicate without being able to confirm known files.
//!
//! * **Keys.** One Argon2id master key per repository (the passphrase and a random salt kept in
//!   `bookx-repo.json`). Two sub-keys are derived with HMAC-SHA256: `enc` (object encryption) and
//!   `id` (blob ids). They never leave memory and are wiped on drop.
//! * **Blob id** = `HMAC-SHA256(id key, plaintext)` for encrypted places (the server cannot test
//!   whether it holds a known file, and two repositories with different passphrases share no
//!   ids); plain `SHA-256` for unencrypted places (so a blob *is* the original file and its name is
//!   its checksum).
//! * **Object** = `BOOKXOBJ` · version · salt(16) · nonce(7) · ChaCha20-Poly1305 STREAM chunks of
//!   64 KiB. Each object has its own key (`HMAC(enc key, "obj" ‖ salt)`) so a random nonce can never
//!   collide across objects under one key. The header and the object's repository path are the
//!   associated data of every chunk: an object copied under another name, a swapped header,
//!   reordered, truncated or flipped chunks all fail authentication. Nothing is compressed, so
//!   there is no decompression to bomb.

use std::io::{Read, Write};

use anyhow::{Result, anyhow, bail};
use chacha20poly1305::aead::Payload;
use chacha20poly1305::aead::generic_array::GenericArray;
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

type HmacSha = Hmac<Sha256>;

pub const OBJ_MAGIC: &[u8; 8] = b"BOOKXOBJ";
pub const OBJ_VERSION: u8 = 1;
pub const OBJ_HEADER: usize = 8 + 1 + 16 + 7;
pub const CHUNK: usize = 64 * 1024;
const TAG: usize = 16;

/// The content size of an encrypted object of `stored` bytes, `None` when no content size gives
/// that length.
pub fn plain_len(stored: u64) -> Option<u64> {
    let body = stored.checked_sub(OBJ_HEADER as u64)?;
    let chunks = body.div_ceil((CHUNK + TAG) as u64).max(1);
    let plain = body.checked_sub(chunks * TAG as u64)?;
    (sealed_len(plain) == stored).then_some(plain)
}

/// The size of the stored object for `plain` bytes of content in an encrypted place.
pub fn sealed_len(plain: u64) -> u64 {
    let chunks = plain.div_ceil(CHUNK as u64).max(1);
    OBJ_HEADER as u64 + plain + chunks * TAG as u64
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        bail!("bad hex");
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|e| anyhow!("{e}")))
        .collect()
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <HmacSha as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// The sub-keys of an encrypted repository.
pub struct Keys {
    enc: [u8; 32],
    idk: [u8; 32],
}

impl Keys {
    pub fn from_master(master: &[u8; 32]) -> Keys {
        Keys {
            enc: hmac(master, &[b"bookx-backup/v1/enc"]),
            idk: hmac(master, &[b"bookx-backup/v1/id"]),
        }
    }

    /// Keys for a new passphrase that keep the id key of `old`: object names stay valid, so a
    /// repository can be re-encrypted in place.
    pub fn from_master_keeping_ids(master: &[u8; 32], old: &Keys) -> Keys {
        Keys {
            enc: hmac(master, &[b"bookx-backup/v1/enc"]),
            idk: old.idk,
        }
    }

    /// The id key, to be stored sealed under the new keys after a passphrase change.
    pub(crate) fn id_key(&self) -> [u8; 32] {
        self.idk
    }

    pub(crate) fn set_id_key(&mut self, idk: [u8; 32]) {
        self.idk = idk;
    }

    fn object_key(&self, salt: &[u8]) -> [u8; 32] {
        hmac(&self.enc, &[b"obj", salt])
    }

    /// A short public tag of the id key, to tell whether a local cache belongs to these keys.
    pub fn fingerprint(&self) -> String {
        hex(&hmac(&self.idk, &[b"fingerprint"])[..8])
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.enc.zeroize();
        self.idk.zeroize();
    }
}

/// Incremental computation of a blob id.
pub enum IdHasher {
    Sha(Sha256),
    Mac(HmacSha),
}

impl IdHasher {
    pub fn update(&mut self, data: &[u8]) {
        match self {
            IdHasher::Sha(h) => h.update(data),
            IdHasher::Mac(m) => m.update(data),
        }
    }

    pub fn finish(self) -> [u8; 32] {
        match self {
            IdHasher::Sha(h) => h.finalize().into(),
            IdHasher::Mac(m) => m.finalize().into_bytes().into(),
        }
    }

    pub fn finish_hex(self) -> String {
        hex(&self.finish())
    }
}

/// `true` when `id` can be a blob id (64 lower-case hex digits).
pub fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Compares an id computed locally with the one a manifest names, in constant time.
pub fn ids_equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

fn random(buf: &mut [u8]) -> Result<()> {
    getrandom::getrandom(buf).map_err(|e| anyhow!("no random numbers available: {e}"))
}

/// How a repository stores its objects.
pub enum Crypto {
    Plain,
    Keyed(Keys),
}

impl Crypto {
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Crypto::Keyed(_))
    }

    pub(crate) fn keys(&self) -> Option<&Keys> {
        match self {
            Crypto::Keyed(k) => Some(k),
            Crypto::Plain => None,
        }
    }

    pub fn id_hasher(&self) -> IdHasher {
        match self {
            Crypto::Plain => IdHasher::Sha(Sha256::new()),
            Crypto::Keyed(k) => {
                IdHasher::Mac(<HmacSha as Mac>::new_from_slice(&k.idk).expect("any key length"))
            }
        }
    }

    pub fn id_of(&self, data: &[u8]) -> String {
        let mut h = self.id_hasher();
        h.update(data);
        h.finish_hex()
    }

    pub fn fingerprint(&self) -> String {
        match self {
            Crypto::Plain => "plain".into(),
            Crypto::Keyed(k) => k.fingerprint(),
        }
    }

    /// Bytes stored for `plain` bytes of content.
    pub fn stored_len(&self, plain: u64) -> u64 {
        match self {
            Crypto::Plain => plain,
            Crypto::Keyed(_) => sealed_len(plain),
        }
    }

    /// A reader over the object to store for the content of `r` (which must deliver exactly
    /// `plain_len` bytes: more or less is an error, a file that changed while it was copied).
    pub fn seal_reader<'a, R: Read + 'a>(
        &self,
        ctx: &str,
        r: R,
        plain_len: u64,
    ) -> Result<Box<dyn Read + 'a>> {
        match self {
            Crypto::Plain => Ok(Box::new(ExactReader::new(r, plain_len))),
            Crypto::Keyed(k) => {
                let mut salt = [0u8; 16];
                let mut nonce = [0u8; 7];
                random(&mut salt)?;
                random(&mut nonce)?;
                let mut header = Vec::with_capacity(OBJ_HEADER);
                header.extend_from_slice(OBJ_MAGIC);
                header.push(OBJ_VERSION);
                header.extend_from_slice(&salt);
                header.extend_from_slice(&nonce);
                let mut key = k.object_key(&salt);
                let enc = EncryptorBE32::from_aead(
                    ChaCha20Poly1305::new(GenericArray::from_slice(&key)),
                    GenericArray::from_slice(&nonce),
                );
                key.zeroize();
                let mut aad = header.clone();
                aad.extend_from_slice(ctx.as_bytes());
                Ok(Box::new(EncReader {
                    inner: r.take(plain_len.saturating_add(1)),
                    expected: plain_len,
                    enc: Some(enc),
                    aad,
                    out: header,
                    pos: 0,
                    cur: Vec::new(),
                    started: false,
                    finished: false,
                    total: 0,
                }))
            }
        }
    }

    /// A writer that turns the stored object back into content for `out`. At most `max_plain`
    /// content bytes are accepted. Call [`OpenWriter::finish`] at the end: it checks that the
    /// object was complete.
    pub fn open_writer<'a>(
        &self,
        ctx: &str,
        out: &'a mut dyn Write,
        max_plain: u64,
    ) -> OpenWriter<'a> {
        OpenWriter {
            out,
            max: max_plain,
            n: 0,
            mode: match self {
                Crypto::Plain => OpenMode::Plain,
                Crypto::Keyed(k) => OpenMode::Enc {
                    enc_key: k.enc,
                    ctx: ctx.to_string(),
                    buf: Vec::new(),
                    dec: None,
                    aad: Vec::new(),
                },
            },
        }
    }

    pub fn seal_bytes(&self, ctx: &str, data: &[u8]) -> Result<Vec<u8>> {
        let mut r = self.seal_reader(ctx, std::io::Cursor::new(data), data.len() as u64)?;
        let mut v = Vec::with_capacity(self.stored_len(data.len() as u64) as usize);
        r.read_to_end(&mut v)?;
        Ok(v)
    }

    pub fn open_bytes(&self, ctx: &str, data: &[u8], max_plain: u64) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut w = self.open_writer(ctx, &mut out, max_plain);
        w.write_all(data)?;
        w.finish()?;
        Ok(out)
    }
}

/// Hands out exactly `n` bytes of `inner`.
struct ExactReader<R> {
    inner: std::io::Take<R>,
    expected: u64,
    seen: u64,
}

impl<R: Read> ExactReader<R> {
    fn new(inner: R, expected: u64) -> Self {
        ExactReader {
            inner: inner.take(expected.saturating_add(1)),
            expected,
            seen: 0,
        }
    }
}

fn changed() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "the file changed while it was being copied",
    )
}

impl<R: Read> Read for ExactReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.seen += n as u64;
        if self.seen > self.expected || (n == 0 && !buf.is_empty() && self.seen != self.expected) {
            return Err(changed());
        }
        Ok(n)
    }
}

struct EncReader<R> {
    inner: std::io::Take<R>,
    expected: u64,
    enc: Option<EncryptorBE32<ChaCha20Poly1305>>,
    aad: Vec<u8>,
    out: Vec<u8>,
    pos: usize,
    cur: Vec<u8>,
    started: bool,
    finished: bool,
    total: u64,
}

impl<R: Read> EncReader<R> {
    fn read_chunk(&mut self) -> std::io::Result<Vec<u8>> {
        let mut v = vec![0u8; CHUNK];
        let mut n = 0;
        while n < CHUNK {
            match self.inner.read(&mut v[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        v.truncate(n);
        self.total += n as u64;
        if self.total > self.expected {
            return Err(changed());
        }
        Ok(v)
    }

    fn next(&mut self) -> std::io::Result<()> {
        if !self.started {
            self.started = true;
            self.cur = self.read_chunk()?;
            return Ok(());
        }
        let next = self.read_chunk()?;
        let payload_aad = self.aad.clone();
        let last = next.is_empty();
        let bad = |_| std::io::Error::other("encryption failed");
        let ct = if last {
            if self.total != self.expected {
                return Err(changed());
            }
            self.finished = true;
            let enc = self
                .enc
                .take()
                .expect("encryptor present until the last chunk");
            enc.encrypt_last(Payload {
                msg: &self.cur,
                aad: &payload_aad,
            })
            .map_err(bad)?
        } else {
            let ct = self
                .enc
                .as_mut()
                .expect("encryptor present until the last chunk")
                .encrypt_next(Payload {
                    msg: &self.cur,
                    aad: &payload_aad,
                })
                .map_err(bad)?;
            self.cur = next;
            ct
        };
        self.out = ct;
        self.pos = 0;
        Ok(())
    }
}

impl<R: Read> Read for EncReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.pos < self.out.len() {
                let n = buf.len().min(self.out.len() - self.pos);
                buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.finished {
                return Ok(0);
            }
            self.next()?;
        }
    }
}

enum OpenMode {
    Plain,
    Enc {
        enc_key: [u8; 32],
        ctx: String,
        buf: Vec<u8>,
        dec: Option<DecryptorBE32<ChaCha20Poly1305>>,
        aad: Vec<u8>,
    },
}

/// See [`Crypto::open_writer`].
pub struct OpenWriter<'a> {
    out: &'a mut dyn Write,
    max: u64,
    n: u64,
    mode: OpenMode,
}

fn damaged() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "wrong passphrase, or the backup is damaged",
    )
}

impl OpenWriter<'_> {
    fn emit(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.n += data.len() as u64;
        if self.n > self.max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the stored object is bigger than it should be",
            ));
        }
        self.out.write_all(data)
    }

    /// The next chunk to decrypt, once the header has been read; `None` when more data is needed.
    fn next_chunk(&mut self, last: bool) -> std::io::Result<Option<(Vec<u8>, bool)>> {
        let OpenMode::Enc {
            enc_key,
            buf,
            dec,
            aad,
            ctx,
        } = &mut self.mode
        else {
            return Ok(None);
        };
        if dec.is_none() {
            if buf.len() < OBJ_HEADER {
                return if last { Err(damaged()) } else { Ok(None) };
            }
            if &buf[..8] != OBJ_MAGIC || buf[8] != OBJ_VERSION {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "this is not a Bookx backup object (or it is from a newer Bookx)",
                ));
            }
            let header: Vec<u8> = buf.drain(..OBJ_HEADER).collect();
            let mut key = hmac(enc_key, &[b"obj", &header[9..25]]);
            *dec = Some(DecryptorBE32::from_aead(
                ChaCha20Poly1305::new(GenericArray::from_slice(&key)),
                GenericArray::from_slice(&header[25..32]),
            ));
            key.zeroize();
            *aad = header;
            aad.extend_from_slice(ctx.as_bytes());
        }
        if buf.len() > CHUNK + TAG {
            Ok(Some((buf.drain(..CHUNK + TAG).collect(), false)))
        } else if last {
            if buf.len() < TAG {
                return Err(damaged());
            }
            Ok(Some((std::mem::take(buf), true)))
        } else {
            Ok(None)
        }
    }

    fn pump(&mut self, last: bool) -> std::io::Result<()> {
        while let Some((chunk, is_last)) = self.next_chunk(last)? {
            let OpenMode::Enc { dec, aad, .. } = &mut self.mode else {
                unreachable!("only encrypted objects have chunks")
            };
            let p = Payload {
                msg: &chunk,
                aad: aad.as_slice(),
            };
            let plain = if is_last {
                dec.take()
                    .expect("decryptor present")
                    .decrypt_last(p)
                    .map_err(|_| damaged())?
            } else {
                dec.as_mut()
                    .expect("decryptor present")
                    .decrypt_next(p)
                    .map_err(|_| damaged())?
            };
            self.emit(&plain)?;
            if is_last {
                break;
            }
        }
        Ok(())
    }

    /// Ends the object; fails when it was cut short. Returns the content bytes written.
    pub fn finish(mut self) -> Result<u64> {
        if matches!(self.mode, OpenMode::Enc { .. }) {
            self.pump(true)?;
        }
        self.out.flush()?;
        Ok(self.n)
    }
}

impl Write for OpenWriter<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        match &mut self.mode {
            OpenMode::Plain => self.emit(data)?,
            OpenMode::Enc { buf, .. } => {
                buf.extend_from_slice(data);
                self.pump(false)?;
            }
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for OpenWriter<'_> {
    fn drop(&mut self) {
        if let OpenMode::Enc { enc_key, .. } = &mut self.mode {
            enc_key.zeroize();
        }
    }
}

/// A random 16-byte value as hex.
pub fn random_hex(bytes: usize) -> Result<String> {
    let mut b = vec![0u8; bytes];
    random(&mut b)?;
    Ok(hex(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crypto() -> Crypto {
        Crypto::Keyed(Keys::from_master(&[7u8; 32]))
    }

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn objects_round_trip_at_every_chunk_boundary() {
        let c = crypto();
        for n in [
            0,
            1,
            100,
            CHUNK - 1,
            CHUNK,
            CHUNK + 1,
            2 * CHUNK,
            2 * CHUNK + 5,
        ] {
            let d = data(n);
            let sealed = c.seal_bytes("blobs/aa/x", &d).unwrap();
            assert_eq!(sealed.len() as u64, sealed_len(n as u64), "n={n}");
            assert_eq!(
                c.open_bytes("blobs/aa/x", &sealed, n as u64).unwrap(),
                d,
                "n={n}"
            );
        }
    }

    #[test]
    fn tampering_truncation_and_renaming_are_detected() {
        let c = crypto();
        let d = data(3 * CHUNK + 10);
        let sealed = c.seal_bytes("snapshots/a", &d).unwrap();
        let max = d.len() as u64;
        // another name
        assert!(c.open_bytes("snapshots/b", &sealed, max).is_err());
        // a flipped byte in each region
        for at in [3, 12, 30, 100, sealed.len() / 2, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[at] ^= 1;
            assert!(
                c.open_bytes("snapshots/a", &bad, max).is_err(),
                "flip at {at}"
            );
        }
        // cut short at a chunk boundary and in the middle
        for cut in [OBJ_HEADER + CHUNK + TAG, sealed.len() - 1, OBJ_HEADER, 5] {
            assert!(
                c.open_bytes("snapshots/a", &sealed[..cut], max).is_err(),
                "cut at {cut}"
            );
        }
        // a longer plain text than allowed
        assert!(c.open_bytes("snapshots/a", &sealed, max - 1).is_err());
        // another key
        let other = Crypto::Keyed(Keys::from_master(&[8u8; 32]));
        assert!(other.open_bytes("snapshots/a", &sealed, max).is_err());
    }

    #[test]
    fn a_source_that_changes_size_is_an_error_not_a_bad_blob() {
        let c = crypto();
        let d = data(1000);
        let mut r = c.seal_reader("x", std::io::Cursor::new(&d), 999).unwrap();
        assert!(r.read_to_end(&mut Vec::new()).is_err());
        let mut r = c.seal_reader("x", std::io::Cursor::new(&d), 1001).unwrap();
        assert!(r.read_to_end(&mut Vec::new()).is_err());
        let p = Crypto::Plain;
        let mut r = p.seal_reader("x", std::io::Cursor::new(&d), 999).unwrap();
        assert!(r.read_to_end(&mut Vec::new()).is_err());
        let mut r = p.seal_reader("x", std::io::Cursor::new(&d), 1000).unwrap();
        let mut v = Vec::new();
        r.read_to_end(&mut v).unwrap();
        assert_eq!(v, d);
    }

    #[test]
    fn keyed_ids_depend_on_the_key_and_plain_ids_are_sha256() {
        let a = crypto().id_of(b"hello");
        let b = Crypto::Keyed(Keys::from_master(&[9u8; 32])).id_of(b"hello");
        assert_ne!(a, b);
        assert_eq!(a, crypto().id_of(b"hello"));
        assert!(valid_id(&a));
        assert_eq!(
            Crypto::Plain.id_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(ids_equal(&a, &a) && !ids_equal(&a, &b));
        assert!(!valid_id("../etc/passwd") && !valid_id(&a.to_uppercase()));
    }
}
