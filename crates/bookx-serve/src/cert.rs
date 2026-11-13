//! Self-signed certificates for sharing over HTTPS on a home network (pure Rust, `rcgen`).
//!
//! A browser or e-reader will warn about a self-signed certificate until the user trusts it; the
//! point is an encrypted connection on the LAN without buying a certificate. Users who own a real
//! certificate pick its files instead (see [`crate::net::tls_config`]).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// File names written by [`write_self_signed`].
pub const CERT_FILE: &str = "bookx-share-cert.pem";
pub const KEY_FILE: &str = "bookx-share-key.pem";

/// A PEM certificate and its PEM private key.
pub struct CertPair {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Generates a self-signed certificate valid for `names` (host names and IP addresses).
/// `localhost` and the loopback addresses are always included.
pub fn generate_self_signed(names: &[String]) -> Result<CertPair> {
    let mut all: Vec<String> = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
    for n in names {
        let n = n.trim();
        if n.is_empty() {
            continue;
        }
        if n.len() > 253 || n.chars().any(|c| c.is_whitespace() || c.is_control()) {
            bail!("not a usable host name: {n:?}");
        }
        if !all.iter().any(|a| a == n) {
            all.push(n.to_string());
        }
    }
    let cert = rcgen::generate_simple_self_signed(all).context("generating the certificate")?;
    Ok(CertPair {
        cert_pem: cert.cert.pem(),
        key_pem: cert.signing_key.serialize_pem(),
    })
}

/// Generates a pair and writes it into `dir` (created if needed); the key file is readable by the
/// owner only. Returns `(certificate path, key path)`.
pub fn write_self_signed(dir: &Path, names: &[String]) -> Result<(PathBuf, PathBuf)> {
    let pair = generate_self_signed(names)?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let cert = dir.join(CERT_FILE);
    let key = dir.join(KEY_FILE);
    std::fs::write(&cert, pair.cert_pem).with_context(|| format!("writing {}", cert.display()))?;
    write_private(&key, pair.key_pem.as_bytes())?;
    Ok((cert, key))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    f.write_all(bytes)?;
    Ok(())
}

/// Why a certificate/key pair cannot be used by the server, `None` when it can.
pub fn check_pair(cert: &Path, key: &Path) -> Option<String> {
    crate::net::tls_config(cert, key)
        .err()
        .map(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-cert-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_generated_pair_loads_into_the_server() {
        let d = tmp();
        let (c, k) = write_self_signed(&d.join("sub"), &["books.lan".into()]).unwrap();
        assert!(c.is_file() && k.is_file());
        assert_eq!(check_pair(&c, &k), None);
        assert!(std::fs::read_to_string(&c)
            .unwrap()
            .starts_with("-----BEGIN CERTIFICATE"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_key_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp();
        let (_, k) = write_self_signed(&d, &[]).unwrap();
        let mode = std::fs::metadata(&k).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn mismatched_or_missing_files_are_explained() {
        let d = tmp();
        let (c1, _k1) = write_self_signed(&d.join("a"), &[]).unwrap();
        let (_c2, k2) = write_self_signed(&d.join("b"), &[]).unwrap();
        assert!(check_pair(&c1, &k2).unwrap().contains("do not match"));
        assert!(check_pair(&d.join("none.pem"), &k2).is_some());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn hostile_names_are_refused_and_pairs_differ() {
        assert!(generate_self_signed(&["a b".into()]).is_err());
        let a = generate_self_signed(&[]).unwrap();
        let b = generate_self_signed(&[]).unwrap();
        assert_ne!(a.key_pem, b.key_pem);
    }
}
