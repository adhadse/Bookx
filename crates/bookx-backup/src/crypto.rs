//! The `.bookxbackup` file: a catalogue snapshot encrypted with a passphrase.
//!
//! **Why this construction?** Off-site copies (a Nextcloud, a NAS that other people share, a cloud
//! drive) should not be readable by whoever runs them. The primitives are the RustCrypto ones,
//! pure Rust and audited more than anything we could write:
//!
//! * **Argon2id** (RFC 9106) turns the passphrase into a 256-bit key; it is memory hard, so a
//!   stolen file cannot be guessed at cheaply on a GPU. The parameters (memory, passes, salt) are
//!   stored in the header, so they can be raised later without breaking old files.
//! * **ChaCha20-Poly1305** in the STREAM construction (`EncryptorBE32`, 64 KiB chunks) encrypts and
//!   authenticates. Every chunk carries its position and the last chunk is flagged, so reordering,
//!   truncation and bit flips are all detected. The header is the associated data of every chunk,
//!   so it cannot be swapped either.
//!
//! I did not pick `age`: its passphrase mode uses scrypt and a fixed format that cannot carry our
//! own metadata (book count, date) in the clear, and the file should be *self-describing*.
//!
//! Layout: `BOOKXBAK` · version byte (1) · header length (u32 big endian) · header (JSON, UTF-8,
//! readable without the passphrase) · encrypted chunks.

use anyhow::{anyhow, bail, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::generic_array::GenericArray;
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::aead::Payload;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 8] = b"BOOKXBAK";
pub const VERSION: u8 = 1;
const CHUNK: usize = 64 * 1024;
const TAG: usize = 16;
const NONCE: usize = 7;
const SALT: usize = 16;
/// The most memory a file may ask the key derivation for.
const MAX_KDF_KIB: u32 = 1024 * 1024;

/// Key derivation cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kdf {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

impl Kdf {
    /// 19 MiB, 2 passes, 1 lane: the RFC 9106 / OWASP recommendation for interactive use.
    pub const DEFAULT: Kdf = Kdf {
        memory_kib: 19 * 1024,
        passes: 2,
        lanes: 1,
    };
    /// For tests only.
    pub const TEST: Kdf = Kdf {
        memory_kib: 64,
        passes: 1,
        lanes: 1,
    };
}

/// What a `.bookxbackup` says about itself, readable without the passphrase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub format: u32,
    pub app: String,
    pub cipher: String,
    pub kdf: String,
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
    pub salt: String,
    pub nonce: String,
    pub chunk: usize,
    pub created: String,
    pub books: usize,
    pub kind: String,
    pub original_name: String,
}

/// What goes into the clear header besides the crypto parameters.
#[derive(Debug, Clone)]
pub struct Meta {
    pub created: String,
    pub books: usize,
    pub kind: String,
    pub original_name: String,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        bail!("bad hex");
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|e| anyhow!("{e}")))
        .collect()
}

pub(crate) fn derive(pass: &str, salt: &[u8], kdf: Kdf) -> Result<[u8; 32]> {
    let params = Params::new(kdf.memory_kib, kdf.passes, kdf.lanes, Some(32))
        .map_err(|e| anyhow!("bad key derivation settings: {e}"))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pass.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

fn random(buf: &mut [u8]) -> Result<()> {
    getrandom::getrandom(buf).map_err(|e| anyhow!("no random numbers available: {e}"))
}

/// True when `data` starts like a `.bookxbackup`.
pub fn is_encrypted(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

/// Encrypts `plain` with the default key derivation.
pub fn encrypt(plain: &[u8], passphrase: &str, meta: &Meta) -> Result<Vec<u8>> {
    encrypt_with(plain, passphrase, meta, Kdf::DEFAULT)
}

pub fn encrypt_with(plain: &[u8], passphrase: &str, meta: &Meta, kdf: Kdf) -> Result<Vec<u8>> {
    if passphrase.is_empty() {
        bail!("the backup passphrase is empty");
    }
    let mut salt = [0u8; SALT];
    let mut nonce = [0u8; NONCE];
    random(&mut salt)?;
    random(&mut nonce)?;
    let header = Header {
        format: VERSION as u32,
        app: format!("Bookx {}", env!("CARGO_PKG_VERSION")),
        cipher: "chacha20poly1305-stream-be32".into(),
        kdf: "argon2id".into(),
        memory_kib: kdf.memory_kib,
        passes: kdf.passes,
        lanes: kdf.lanes,
        salt: hex(&salt),
        nonce: hex(&nonce),
        chunk: CHUNK,
        created: meta.created.clone(),
        books: meta.books,
        kind: meta.kind.clone(),
        original_name: meta.original_name.clone(),
    };
    let hjson = serde_json::to_vec(&header)?;
    let key = derive(passphrase, &salt, kdf)?;
    let cipher = ChaCha20Poly1305::new(GenericArray::from_slice(&key));
    let mut enc = EncryptorBE32::from_aead(cipher, GenericArray::from_slice(&nonce));

    let mut out = Vec::with_capacity(plain.len() + plain.len() / CHUNK * TAG + hjson.len() + 64);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(hjson.len() as u32).to_be_bytes());
    out.extend_from_slice(&hjson);
    let mut chunks = plain.chunks(CHUNK).peekable();
    if chunks.peek().is_none() {
        let ct = enc
            .encrypt_last(Payload {
                msg: &[],
                aad: &hjson,
            })
            .map_err(|_| anyhow!("encryption failed"))?;
        out.extend_from_slice(&ct);
        return Ok(out);
    }
    while let Some(c) = chunks.next() {
        let p = Payload {
            msg: c,
            aad: &hjson,
        };
        if chunks.peek().is_some() {
            out.extend(
                enc.encrypt_next(p)
                    .map_err(|_| anyhow!("encryption failed"))?,
            );
        } else {
            out.extend(
                enc.encrypt_last(p)
                    .map_err(|_| anyhow!("encryption failed"))?,
            );
            break;
        }
    }
    Ok(out)
}

/// Splits the file into its header and the encrypted part. Needs no passphrase.
pub fn read_header(data: &[u8]) -> Result<(Header, &[u8], &[u8])> {
    if !is_encrypted(data) {
        bail!("this is not a Bookx encrypted backup");
    }
    if data.len() < 13 {
        bail!("the backup file is cut short");
    }
    if data[8] != VERSION {
        bail!(
            "this backup was made by a newer Bookx (format {}); update Bookx to read it",
            data[8]
        );
    }
    let n = u32::from_be_bytes(data[9..13].try_into().unwrap()) as usize;
    if n > 64 * 1024 || data.len() < 13 + n {
        bail!("the backup file's header is damaged");
    }
    let raw = &data[13..13 + n];
    let header: Header =
        serde_json::from_slice(raw).context("the backup file's header is damaged")?;
    Ok((header, raw, &data[13 + n..]))
}

/// The clear-text description of an encrypted backup.
pub fn describe(data: &[u8]) -> Result<Header> {
    read_header(data).map(|h| h.0)
}

/// Decrypts a `.bookxbackup`. A wrong passphrase and a damaged file give different messages
/// where they can be told apart (they cannot always be: a flipped bit in the first chunk looks
/// like a wrong passphrase).
pub fn decrypt(data: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let (h, raw, body) = read_header(data)?;
    if h.kdf != "argon2id" || h.cipher != "chacha20poly1305-stream-be32" {
        bail!(
            "this backup uses an unknown method ({}, {})",
            h.kdf,
            h.cipher
        );
    }
    if h.memory_kib > MAX_KDF_KIB || h.passes > 64 || h.lanes == 0 || h.lanes > 16 {
        bail!("the backup asks for unreasonable key settings; refusing to open it");
    }
    let salt = unhex(&h.salt)?;
    let nonce = unhex(&h.nonce)?;
    if nonce.len() != NONCE || h.chunk != CHUNK {
        bail!("the backup file's header is damaged");
    }
    let key = derive(
        passphrase,
        &salt,
        Kdf {
            memory_kib: h.memory_kib,
            passes: h.passes,
            lanes: h.lanes,
        },
    )?;
    let cipher = ChaCha20Poly1305::new(GenericArray::from_slice(&key));
    let mut dec = DecryptorBE32::from_aead(cipher, GenericArray::from_slice(&nonce));
    let wrong = || anyhow!("wrong passphrase, or the backup file is damaged");
    if body.len() < TAG {
        bail!("the backup file is cut short");
    }
    let mut out = Vec::with_capacity(body.len());
    let mut chunks = body.chunks(CHUNK + TAG).peekable();
    let mut first = true;
    while let Some(c) = chunks.next() {
        let p = Payload { msg: c, aad: raw };
        let r = if chunks.peek().is_some() {
            dec.decrypt_next(p)
        } else {
            // the stream must end with the chunk flagged as last, or the file was cut
            let r = dec.decrypt_last(p);
            out.extend(r.map_err(|_| {
                if first {
                    wrong()
                } else {
                    anyhow!("the backup file is damaged or cut short")
                }
            })?);
            return Ok(out);
        };
        out.extend(r.map_err(|_| {
            if first {
                wrong()
            } else {
                anyhow!("the backup file is damaged")
            }
        })?);
        first = false;
    }
    Err(anyhow!("the backup file is cut short"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta {
            created: "2026-11-12T10:00:00Z".into(),
            books: 42,
            kind: "manual".into(),
            original_name: "bookx-20261112T100000Z-n42-manual.db".into(),
        }
    }

    fn sample(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn round_trips_empty_small_exact_and_multi_chunk() {
        for n in [
            0,
            1,
            100,
            CHUNK - 1,
            CHUNK,
            CHUNK + 1,
            3 * CHUNK,
            3 * CHUNK + 17,
        ] {
            let plain = sample(n);
            let enc = encrypt_with(&plain, "correct horse", &meta(), Kdf::TEST).unwrap();
            assert!(is_encrypted(&enc));
            assert_eq!(decrypt(&enc, "correct horse").unwrap(), plain, "size {n}");
        }
    }

    #[test]
    fn the_header_is_readable_without_the_passphrase() {
        let enc = encrypt_with(b"secret catalogue", "pw", &meta(), Kdf::TEST).unwrap();
        let h = describe(&enc).unwrap();
        assert_eq!(h.books, 42);
        assert_eq!(h.kind, "manual");
        assert_eq!(h.kdf, "argon2id");
        assert!(h.app.starts_with("Bookx "));
        assert!(!String::from_utf8_lossy(&enc).contains("secret catalogue"));
    }

    #[test]
    fn a_wrong_passphrase_is_refused() {
        let enc = encrypt_with(b"data", "right", &meta(), Kdf::TEST).unwrap();
        let e = decrypt(&enc, "wrong").unwrap_err().to_string();
        assert!(e.contains("wrong passphrase"), "{e}");
    }

    #[test]
    fn each_file_gets_its_own_salt_and_nonce() {
        let a = encrypt_with(b"same", "pw", &meta(), Kdf::TEST).unwrap();
        let b = encrypt_with(b"same", "pw", &meta(), Kdf::TEST).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn tampering_truncation_and_reordering_are_detected() {
        let plain = sample(3 * CHUNK + 5);
        let enc = encrypt_with(&plain, "pw", &meta(), Kdf::TEST).unwrap();
        // flip a bit in the last chunk
        let mut t = enc.clone();
        let n = t.len();
        t[n - 3] ^= 1;
        assert!(decrypt(&t, "pw").is_err());
        // cut off the last chunk
        let cut = enc.len() - (5 + TAG);
        assert!(decrypt(&enc[..cut], "pw").is_err());
        // cut at a chunk boundary
        let (_, _, body) = read_header(&enc).unwrap();
        let boundary = enc.len() - body.len() + CHUNK + TAG;
        assert!(decrypt(&enc[..boundary], "pw").is_err());
        // swap two chunks
        let start = enc.len() - body.len();
        let mut s = enc.clone();
        let (a, b) = (start, start + CHUNK + TAG);
        for i in 0..CHUNK + TAG {
            s.swap(a + i, b + i);
        }
        assert!(decrypt(&s, "pw").is_err());
        // a changed header (the book count) is caught because it is authenticated
        let text = String::from_utf8_lossy(&enc[13..]).to_string();
        let i = text.find("\"books\":42").unwrap();
        let mut h = enc.clone();
        h[13 + i + 9] = b'9';
        assert!(decrypt(&h, "pw").is_err());
    }

    #[test]
    fn files_that_are_not_ours_or_too_new_are_explained() {
        assert!(decrypt(b"SQLite format 3\0....", "pw")
            .unwrap_err()
            .to_string()
            .contains("not a Bookx"));
        let mut enc = encrypt_with(b"x", "pw", &meta(), Kdf::TEST).unwrap();
        enc[8] = 9;
        assert!(decrypt(&enc, "pw")
            .unwrap_err()
            .to_string()
            .contains("newer"));
    }

    #[test]
    fn a_file_cannot_ask_for_gigabytes_of_key_derivation() {
        let enc = encrypt_with(b"x", "pw", &meta(), Kdf::TEST).unwrap();
        let text = String::from_utf8(
            enc[13..13 + u32::from_be_bytes(enc[9..13].try_into().unwrap()) as usize].to_vec(),
        )
        .unwrap();
        let evil = text.replace("\"memory_kib\":64", "\"memory_kib\":99999999");
        let mut f = enc[..9].to_vec();
        f.extend((evil.len() as u32).to_be_bytes());
        f.extend(evil.as_bytes());
        f.extend(&enc[13 + text.len()..]);
        assert!(decrypt(&f, "pw")
            .unwrap_err()
            .to_string()
            .contains("unreasonable"));
    }

    #[test]
    fn an_empty_passphrase_is_not_allowed() {
        assert!(encrypt_with(b"x", "", &meta(), Kdf::TEST).is_err());
    }
}
