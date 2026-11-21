//! How the sync server keeps the account key on disk.
//!
//! The KOSync protocol makes the *client* send `x-auth-key`, the MD5 hex of the password, with every
//! request. That is a protocol limit Bookx cannot change: the key itself is the credential, and MD5
//! is fast. What Bookx controls is what the *server* writes down: not the key, but a salted
//! Argon2id hash of it (`$a2id$m,t,p$salt$hash`), so a stolen `kosync.json` does not hand out
//! working logins and cracking a key costs real memory and time per guess. Checks are constant
//! time. Verified keys are remembered in memory only as an HMAC under a random per-process key, so
//! the slow hash is not paid on every request of a sync.
//!
//! Records written by older versions hold the raw key; [`is_hashed`] tells them apart and the
//! stores upgrade them when they open the file.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use argon2::{Algorithm, Argon2, Params, Version};
use hmac::{Hmac, Mac};
use sha2::Sha256;

const PREFIX: &str = "$a2id$";
const MAX_KIB: u32 = 256 * 1024;
const MAX_PASSES: u32 = 10;
const MAX_LANES: u32 = 8;
const CACHE_MAX: usize = 256;

fn params() -> (u32, u32, u32) {
    if cfg!(test) {
        (64, 1, 1)
    } else {
        (19 * 1024, 2, 1)
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok())
        .collect()
}

fn slow(key: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Option<[u8; 32]> {
    let params = Params::new(m, t, p, Some(32)).ok()?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(key.as_bytes(), salt, &mut out)
        .ok()?;
    Some(out)
}

/// True when `stored` is a hash made by [`hash_key`] (not a raw key of an older version).
pub fn is_hashed(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

/// The record to write for `key`.
pub fn hash_key(key: &str) -> String {
    let mut salt = [0u8; 16];
    let _ = getrandom::getrandom(&mut salt);
    let (m, t, p) = params();
    let h = slow(key, &salt, m, t, p).unwrap_or([0u8; 32]);
    format!("{PREFIX}{m},{t},{p}${}${}", hex(&salt), hex(&h))
}

type Record = (u32, u32, u32, Vec<u8>, Vec<u8>);
type Cache = (Mutex<HashSet<[u8; 32]>>, [u8; 32]);

fn parse(stored: &str) -> Option<Record> {
    let rest = stored.strip_prefix(PREFIX)?;
    let mut it = rest.split('$');
    let (cost, salt, hash) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    let mut c = cost.split(',');
    let (m, t, p): (u32, u32, u32) = (
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
    );
    if c.next().is_some()
        || !(8..=MAX_KIB).contains(&m)
        || !(1..=MAX_PASSES).contains(&t)
        || !(1..=MAX_LANES).contains(&p)
    {
        return None;
    }
    let (salt, hash) = (unhex(salt)?, unhex(hash)?);
    if salt.len() < 8 || salt.len() > 64 || hash.len() != 32 {
        return None;
    }
    Some((m, t, p, salt, hash))
}

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| {
        let mut k = [0u8; 32];
        let _ = getrandom::getrandom(&mut k);
        (Mutex::new(HashSet::new()), k)
    })
}

fn tag(user: &str, stored: &str, key: &str) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&cache().1).expect("hmac key");
    for part in [user, stored, key] {
        m.update(&(part.len() as u64).to_be_bytes());
        m.update(part.as_bytes());
    }
    m.finalize().into_bytes().into()
}

/// Does `key` match the record? Constant time in the hash comparison; an unreadable record never matches.
pub fn verify(user: &str, stored: &str, key: &str) -> bool {
    let t = tag(user, stored, key);
    if cache().0.lock().map(|c| c.contains(&t)).unwrap_or(false) {
        return true;
    }
    let Some((m, tt, p, salt, hash)) = parse(stored) else {
        return false;
    };
    let Some(got) = slow(key, &salt, m, tt, p) else {
        return false;
    };
    let ok = crate::http::constant_time_eq(&got, &hash);
    if ok {
        if let Ok(mut c) = cache().0.lock() {
            if c.len() >= CACHE_MAX {
                c.clear();
            }
            c.insert(t);
        }
    }
    ok
}

/// Spends the time of a check for a user that does not exist, so names cannot be told apart by timing.
pub fn dummy_verify(key: &str) {
    let (m, t, p) = params();
    let _ = slow(key, b"bookx-dummy-salt", m, t, p);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_verifies_only_its_key_and_hides_it() {
        let h = hash_key("5f4dcc3b5aa765d61d8327deb882cf99");
        assert!(is_hashed(&h) && !h.contains("5f4dcc3b"));
        assert!(verify("u", &h, "5f4dcc3b5aa765d61d8327deb882cf99"));
        assert!(!verify("u", &h, "5f4dcc3b5aa765d61d8327deb882cf98"));
        assert_ne!(h, hash_key("5f4dcc3b5aa765d61d8327deb882cf99"), "salted");
    }

    #[test]
    fn a_cached_success_is_not_shared_between_users_or_records() {
        let h = hash_key("k");
        assert!(verify("alice", &h, "k"));
        assert!(!verify("bob", &hash_key("other"), "k"));
        assert!(!verify("alice", "$a2id$garbage", "k"));
    }

    #[test]
    fn hostile_records_are_refused_not_run() {
        for bad in [
            "$a2id$4294967295,3,1$00112233445566778899aabbccddeeff$00",
            "$a2id$64,1,1$zz$zz",
            "$a2id$64,1,1$0011$0011",
            "$a2id$64,99,1$00112233445566778899aabbccddeeff$00",
            "$a2id$",
            "",
        ] {
            assert!(!verify("u", bad, "k"), "{bad}");
        }
    }
}
