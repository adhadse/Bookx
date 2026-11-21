//! Plugin trust (X3, security G10): what a plugin may do in plain terms, a fingerprint to pin what the
//! user allowed, and a detached signature file.
//!
//! * [`capabilities`] lists a manifest's permissions as [`Capability`] values; the application words
//!   them (and translates them) in the *capability sheet* shown before a plugin is switched on.
//! * [`fingerprint`] is the SHA-256 of `plugin.toml` and the `.wasm` file. The manager stores it when
//!   the user allows the plugin; a plugin whose files changed afterwards is not run until it is allowed
//!   again (see [`crate::PluginManager::trust_state`]).
//! * `plugin.sig` is an optional detached signature (ed25519) over [`signed_message`]. **Nothing in the
//!   tree can verify ed25519 yet** (no such crate is a dependency), so [`UnsupportedVerifier`] answers
//!   "cannot check" and a signed plugin is shown as *signed, not verified*. The format, the parsing with
//!   limits and the verifier trait are here so a real verifier is one `impl` away. Policy (decision):
//!   unsigned plugins are allowed, with a warning in the sheet.

use std::path::Path;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::manifest::{Manifest, Permission};

/// Name of the detached signature file next to `plugin.toml`.
pub const SIGNATURE_FILE: &str = "plugin.sig";
/// First line of a signature file.
pub const SIGNATURE_MAGIC: &str = "bookx-plugin-signature 1";
/// A signature file larger than this is refused unread (a real one has about 220 bytes).
pub const MAX_SIGNATURE_BYTES: u64 = 1024;
/// Biggest `plugin.toml` / `.wasm` the fingerprint reads (the host refuses larger modules anyway).
pub const MAX_PLUGIN_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// One thing a plugin is allowed to do, independent of the words used to show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// `editor.read`: reads the book open in the editor.
    ReadBook,
    /// `editor.write`: changes the book open in the editor.
    ChangeBook,
    /// `network:<host>`: connects to this host.
    Network(String),
}

/// The manifest's permissions as capabilities, in manifest order. A plugin with none can only compute.
pub fn capabilities(m: &Manifest) -> Vec<Capability> {
    m.permissions
        .iter()
        .map(|p| match p {
            Permission::EditorRead => Capability::ReadBook,
            Permission::EditorWrite => Capability::ChangeBook,
            Permission::Network(h) => Capability::Network(h.clone()),
        })
        .collect()
}

/// SHA-256 (lower-case hex) over the manifest file and the module it names.
pub fn fingerprint(dir: &Path, m: &Manifest) -> Result<String> {
    let mut h = Sha256::new();
    for (tag, path) in [
        (b"toml".as_slice(), dir.join("plugin.toml")),
        (b"wasm".as_slice(), m.wasm_path(dir)),
    ] {
        let data = read_limited(&path, MAX_PLUGIN_FILE_BYTES)?;
        h.update(tag);
        h.update((data.len() as u64).to_le_bytes());
        h.update(&data);
    }
    Ok(hex(&h.finalize()))
}

fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>> {
    let len = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    if len > max {
        bail!("{} is larger than {max} bytes", path.display());
    }
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex(s: &str, len: usize) -> Option<Vec<u8>> {
    if s.len() != len * 2 || !s.is_ascii() {
        return None;
    }
    (0..len)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// What the signature covers: a domain tag and the two file hashes, so a signature for one plugin
/// cannot be moved onto another module.
pub fn signed_message(dir: &Path, m: &Manifest) -> Result<Vec<u8>> {
    let toml = Sha256::digest(read_limited(
        &dir.join("plugin.toml"),
        MAX_PLUGIN_FILE_BYTES,
    )?);
    let wasm = Sha256::digest(read_limited(&m.wasm_path(dir), MAX_PLUGIN_FILE_BYTES)?);
    let mut msg = b"bookx-plugin-v1\0".to_vec();
    msg.extend_from_slice(&toml);
    msg.extend_from_slice(&wasm);
    Ok(msg)
}

/// A parsed `plugin.sig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureFile {
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
}

impl SignatureFile {
    /// Parses the text form:
    /// ```text
    /// bookx-plugin-signature 1
    /// key: <64 hex digits>
    /// signature: <128 hex digits>
    /// ```
    /// Unknown lines, duplicate keys, other lengths and anything over [`MAX_SIGNATURE_BYTES`] are errors.
    pub fn parse(text: &str) -> Result<SignatureFile> {
        if text.len() as u64 > MAX_SIGNATURE_BYTES {
            bail!("the signature file is too large");
        }
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(SIGNATURE_MAGIC) {
            bail!("not a Bookx plugin signature (first line must be `{SIGNATURE_MAGIC}`)");
        }
        let (mut key, mut sig) = (None, None);
        for line in lines.filter(|l| !l.trim().is_empty()) {
            let Some((k, v)) = line.split_once(':') else {
                bail!("unexpected line in the signature file");
            };
            let slot = match k.trim() {
                "key" => &mut key,
                "signature" => &mut sig,
                other => bail!(
                    "unknown field `{}` in the signature file",
                    other.escape_debug()
                ),
            };
            if slot.is_some() {
                bail!("`{}` appears twice in the signature file", k.trim());
            }
            *slot = Some(v.trim().to_string());
        }
        let key = key.context("the signature file has no `key`")?;
        let sig = sig.context("the signature file has no `signature`")?;
        let public_key: [u8; 32] = unhex(&key, 32)
            .context("`key` must be 64 hex digits")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("bad key length"))?;
        let signature: [u8; 64] = unhex(&sig, 64)
            .context("`signature` must be 128 hex digits")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("bad signature length"))?;
        Ok(SignatureFile {
            public_key,
            signature,
        })
    }

    /// Short id of the signing key (first 8 bytes of its SHA-256), for the sheet and the trust list.
    pub fn key_id(&self) -> String {
        hex(&Sha256::digest(self.public_key)[..8])
    }
}

/// Checks an ed25519 signature. `None` means "this build cannot check signatures".
pub trait SignatureVerifier {
    fn verify(&self, key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> Option<bool>;
}

/// The verifier of this build: there is no ed25519 implementation in the dependency tree.
pub struct UnsupportedVerifier;

impl SignatureVerifier for UnsupportedVerifier {
    fn verify(&self, _: &[u8; 32], _: &[u8], _: &[u8; 64]) -> Option<bool> {
        None
    }
}

/// What the capability sheet says about the plugin's origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureStatus {
    /// No `plugin.sig`. Allowed, with a warning.
    Unsigned,
    /// `plugin.sig` is present but unreadable or malformed.
    Malformed(String),
    /// Present and well formed, but this build cannot check it. Shown as "signed, not verified".
    Unverified { key_id: String },
    /// Verified against the signing key.
    Valid { key_id: String },
    /// The signature does not match these files.
    Invalid { key_id: String },
}

impl SignatureStatus {
    /// Whether the sheet shows the "unverified plugin" warning.
    pub fn warns(&self) -> bool {
        !matches!(self, SignatureStatus::Valid { .. })
    }
}

/// Reads `plugin.sig` in `dir` and checks it with `verifier`.
pub fn check_signature(
    dir: &Path,
    m: &Manifest,
    verifier: &dyn SignatureVerifier,
) -> SignatureStatus {
    let path = dir.join(SIGNATURE_FILE);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SignatureStatus::Unsigned,
        Err(e) => return SignatureStatus::Malformed(e.to_string()),
    };
    if !meta.is_file() || meta.len() > MAX_SIGNATURE_BYTES {
        return SignatureStatus::Malformed("not a small regular file".into());
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return SignatureStatus::Malformed(e.to_string()),
    };
    let file = match SignatureFile::parse(&text) {
        Ok(f) => f,
        Err(e) => return SignatureStatus::Malformed(format!("{e:#}")),
    };
    let key_id = file.key_id();
    let msg = match signed_message(dir, m) {
        Ok(m) => m,
        Err(e) => return SignatureStatus::Malformed(format!("{e:#}")),
    };
    match verifier.verify(&file.public_key, &msg, &file.signature) {
        None => SignatureStatus::Unverified { key_id },
        Some(true) => SignatureStatus::Valid { key_id },
        Some(false) => SignatureStatus::Invalid { key_id },
    }
}

/// Whether a plugin the user allowed earlier may still run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    /// The user never allowed this plugin (the sheet has to be shown).
    NeverAsked,
    /// Allowed, and the files are the ones that were allowed.
    Trusted,
    /// Allowed once, but `plugin.toml` or the module changed since: ask again.
    Changed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(perms: &str) -> Manifest {
        Manifest::parse(&format!(
            "id = \"t.p\"\nname = \"P\"\nversion = \"1.0\"\napi_version = 2\nkind = \"editor-action\"\npermissions = [{perms}]\n[[action]]\nid = \"a\"\nlabel = \"A\"\n"
        ))
        .unwrap()
    }

    fn folder(tag: &str, wasm: &[u8]) -> (std::path::PathBuf, Manifest) {
        let d = std::env::temp_dir().join(format!("bookx-trust-{tag}-{}", uuid_like()));
        std::fs::create_dir_all(&d).unwrap();
        let m = manifest("\"editor.read\"");
        std::fs::write(d.join("plugin.toml"), "x").unwrap();
        std::fs::write(d.join("plugin.wasm"), wasm).unwrap();
        (d, m)
    }

    fn uuid_like() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!(
            "{}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    }

    #[test]
    fn capabilities_follow_the_manifest() {
        let m = manifest("\"editor.read\", \"editor.write\"");
        assert_eq!(
            capabilities(&m),
            vec![Capability::ReadBook, Capability::ChangeBook]
        );
        assert!(capabilities(&manifest("")).is_empty());
    }

    #[test]
    fn the_fingerprint_changes_with_either_file() {
        let (d, m) = folder("fp", b"\0asm1");
        let a = fingerprint(&d, &m).unwrap();
        assert_eq!(a.len(), 64);
        assert_eq!(a, fingerprint(&d, &m).unwrap());
        std::fs::write(d.join("plugin.wasm"), b"\0asm2").unwrap();
        let b = fingerprint(&d, &m).unwrap();
        assert_ne!(a, b);
        std::fs::write(d.join("plugin.toml"), "y").unwrap();
        assert_ne!(b, fingerprint(&d, &m).unwrap());
    }

    #[test]
    fn the_fingerprint_does_not_confuse_the_boundary_between_files() {
        let (d, m) = folder("bound", b"bc");
        std::fs::write(d.join("plugin.toml"), "a").unwrap();
        let x = fingerprint(&d, &m).unwrap();
        std::fs::write(d.join("plugin.toml"), "ab").unwrap();
        std::fs::write(d.join("plugin.wasm"), b"c").unwrap();
        assert_ne!(x, fingerprint(&d, &m).unwrap());
    }

    const KEY: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

    fn sig_text(key: &str, sig: &str) -> String {
        format!("{SIGNATURE_MAGIC}\nkey: {key}\nsignature: {sig}\n")
    }

    #[test]
    fn a_signature_file_parses_and_is_strict() {
        let sig = "ab".repeat(64);
        let f = SignatureFile::parse(&sig_text(KEY, &sig)).unwrap();
        assert_eq!(f.public_key[0], 1);
        assert_eq!(f.signature[63], 0xab);
        assert_eq!(f.key_id().len(), 16);
        for bad in [
            "".to_string(),
            "wrong\n".to_string(),
            sig_text("zz", &sig),
            sig_text(KEY, "ab"),
            sig_text(&KEY.to_uppercase().replace('A', "G"), &sig),
            format!("{}extra: 1\n", sig_text(KEY, &sig)),
            format!("{}key: {KEY}\n", sig_text(KEY, &sig)),
            format!("{SIGNATURE_MAGIC}\nkey: {KEY}\n"),
            format!("{}{}", sig_text(KEY, &sig), "#".repeat(2000)),
            sig_text("é".repeat(32).as_str(), &sig),
        ] {
            assert!(SignatureFile::parse(&bad).is_err(), "accepted {bad:?}");
        }
    }

    struct Always(bool);
    impl SignatureVerifier for Always {
        fn verify(&self, _: &[u8; 32], _: &[u8], _: &[u8; 64]) -> Option<bool> {
            Some(self.0)
        }
    }

    #[test]
    fn signature_status_by_verifier() {
        let (d, m) = folder("sig", b"w");
        assert_eq!(
            check_signature(&d, &m, &UnsupportedVerifier),
            SignatureStatus::Unsigned
        );
        std::fs::write(d.join(SIGNATURE_FILE), sig_text(KEY, &"cd".repeat(64))).unwrap();
        assert!(matches!(
            check_signature(&d, &m, &UnsupportedVerifier),
            SignatureStatus::Unverified { .. }
        ));
        let ok = check_signature(&d, &m, &Always(true));
        assert!(matches!(ok, SignatureStatus::Valid { .. }));
        assert!(!ok.warns());
        assert!(matches!(
            check_signature(&d, &m, &Always(false)),
            SignatureStatus::Invalid { .. }
        ));
        assert!(SignatureStatus::Unsigned.warns());
        std::fs::write(d.join(SIGNATURE_FILE), "garbage").unwrap();
        assert!(matches!(
            check_signature(&d, &m, &Always(true)),
            SignatureStatus::Malformed(_)
        ));
        std::fs::write(d.join(SIGNATURE_FILE), vec![b'x'; 5000]).unwrap();
        assert!(matches!(
            check_signature(&d, &m, &Always(true)),
            SignatureStatus::Malformed(_)
        ));
    }

    #[test]
    fn the_signed_message_binds_both_files() {
        let (d, m) = folder("msg", b"w1");
        let a = signed_message(&d, &m).unwrap();
        std::fs::write(d.join("plugin.wasm"), b"w2").unwrap();
        assert_ne!(a, signed_message(&d, &m).unwrap());
        assert!(a.starts_with(b"bookx-plugin-v1\0"));
    }
}
