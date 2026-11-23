#![forbid(unsafe_code)]
//! Passwords in the Secret Service instead of the settings file.
//!
//! **Why `oo7`?** It speaks the freedesktop Secret Service API (GNOME Keyring, KWallet, KeePassXC)
//! and, inside a Flatpak, the `org.freedesktop.portal.Secrets` portal with an encrypted file
//! keyring, with no C library (`libsecret`) in the build. It is the library GNOME's own apps use.
//! The async runtime is the small `async-io` one that `zbus` already brings; nothing here needs
//! tokio.
//!
//! Without a keyring the values go to an encrypted file ([`FileStore`], master password) or stay in memory;
//! they are never written in clear text.
//!
//! Everything goes through the [`SecretStore`] trait, so the logic around it (migrating plain text
//! values, falling back, caching) is tested with [`MemoryStore`] and no desktop session.
//!
//! Secrets are never written to a log: no function here prints a value, and the error texts
//! carry only keys and D-Bus messages.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

mod file;
pub use file::{
    FileStore, Kdf, OpenError, check_passphrase, kdf_is_reasonable, probe, restrict_dir,
    restrict_file,
};

use anyhow::{Result, anyhow};

/// Where passwords live. Keys are short names such as `share-password`.
pub trait SecretStore: Send + Sync {
    /// Short name shown to the user: "the system keyring".
    fn describe(&self) -> String;
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
    /// False when there is no keyring behind this store.
    fn available(&self) -> bool {
        true
    }
}

/// Keeps secrets in memory only: for tests, and for the UI test suite (`BOOKX_SECRETS=memory`).
#[derive(Default)]
pub struct MemoryStore {
    items: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    /// The keys held, sorted. Values are not exposed.
    pub fn keys(&self) -> Vec<String> {
        let mut k: Vec<String> = self.items.lock().unwrap().keys().cloned().collect();
        k.sort();
        k
    }
}

impl SecretStore for MemoryStore {
    fn describe(&self) -> String {
        "a temporary in-memory keyring".into()
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.items.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.items
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.items.lock().unwrap().remove(key);
        Ok(())
    }
}

/// No keyring: every call fails with the reason. The caller falls back to the settings file.
pub struct Unavailable(pub String);

impl SecretStore for Unavailable {
    fn available(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "no keyring".into()
    }
    fn get(&self, _: &str) -> Result<Option<String>> {
        Err(anyhow!("{}", self.0))
    }
    fn set(&self, _: &str, _: &str) -> Result<()> {
        Err(anyhow!("{}", self.0))
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(anyhow!("{}", self.0))
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
const APP_ATTR: (&str, &str) = ("application", "com.adhadse.Bookx");

/// The Secret Service (or the Flatpak portal) through `oo7`. Not built on macOS and Windows.
#[cfg(not(any(target_os = "macos", windows)))]
pub struct Oo7Store;

#[cfg(not(any(target_os = "macos", windows)))]
impl Oo7Store {
    /// Connects once to see whether a keyring answers.
    pub fn connect() -> Result<Oo7Store> {
        async_io::block_on(async {
            oo7::Keyring::new()
                .await
                .map(|_| Oo7Store)
                .map_err(|e| anyhow!("no keyring is available ({e})"))
        })
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn attrs(key: &str) -> Vec<(&str, &str)> {
    vec![APP_ATTR, ("key", key)]
}

#[cfg(not(any(target_os = "macos", windows)))]
impl SecretStore for Oo7Store {
    fn describe(&self) -> String {
        "the system keyring".into()
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        async_io::block_on(async {
            let ring = oo7::Keyring::new().await.map_err(|e| anyhow!("{e}"))?;
            let found = ring
                .search_items(&attrs(key))
                .await
                .map_err(|e| anyhow!("{e}"))?;
            match found.first() {
                None => Ok(None),
                Some(item) => {
                    let s = item.secret().await.map_err(|e| anyhow!("{e}"))?;
                    Ok(Some(String::from_utf8_lossy(s.as_bytes()).into_owned()))
                }
            }
        })
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        async_io::block_on(async {
            let ring = oo7::Keyring::new().await.map_err(|e| anyhow!("{e}"))?;
            ring.create_item(&format!("Bookx: {key}"), &attrs(key), value, true)
                .await
                .map_err(|e| anyhow!("{e}"))
        })
    }

    fn delete(&self, key: &str) -> Result<()> {
        async_io::block_on(async {
            let ring = oo7::Keyring::new().await.map_err(|e| anyhow!("{e}"))?;
            ring.delete(&attrs(key)).await.map_err(|e| anyhow!("{e}"))
        })
    }
}

/// Service name under which the native stores file Bookx's items.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
const NATIVE_SERVICE: &str = "com.adhadse.Bookx";

/// macOS Keychain and Windows Credential Manager through the `keyring` crate (safe API).
/// Compiled only on those systems; it has not been run on real hardware yet (docs/macos-port.md,
/// docs/windows-port.md).
#[cfg(any(target_os = "macos", windows))]
pub struct NativeStore {
    label: &'static str,
}

#[cfg(any(target_os = "macos", windows))]
impl NativeStore {
    /// Writes and reads back a probe item so a locked or missing store is noticed at start-up.
    pub fn connect() -> Result<NativeStore> {
        let label = if cfg!(target_os = "macos") {
            "the macOS Keychain"
        } else {
            "the Windows Credential Manager"
        };
        let store = NativeStore { label };
        store
            .set("probe", "1")
            .map_err(|e| anyhow!("no keyring is available ({e})"))?;
        store.delete("probe")?;
        Ok(store)
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(NATIVE_SERVICE, key).map_err(|e| anyhow!("{e}"))
    }
}

#[cfg(any(target_os = "macos", windows))]
impl SecretStore for NativeStore {
    fn describe(&self) -> String {
        self.label.into()
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow!("{e}")),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| anyhow!("{e}"))
    }

    fn delete(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow!("{e}")),
        }
    }
}

/// Which system store `open_default` tries: the Secret Service on Linux and the other Unixes, the
/// Keychain on macOS, the Credential Manager on Windows.
pub fn system_store_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "keychain"
    } else if cfg!(windows) {
        "credential-manager"
    } else {
        "secret-service"
    }
}

fn open_system_store() -> Box<dyn SecretStore> {
    #[cfg(any(target_os = "macos", windows))]
    {
        match NativeStore::connect() {
            Ok(s) => Box::new(s),
            Err(e) => Box::new(Unavailable(format!("{e:#}"))),
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        match Oo7Store::connect() {
            Ok(s) => Box::new(s),
            Err(e) => Box::new(Unavailable(format!("{e:#}"))),
        }
    }
}

/// The store to use: `BOOKX_SECRETS=memory` (tests), `none` (pretend there is no keyring), else
/// the Secret Service, and when it does not answer, [`Unavailable`].
pub fn open_default() -> Box<dyn SecretStore> {
    open_for(std::env::var("BOOKX_SECRETS").ok().as_deref())
}

/// [`open_default`] with the value of `BOOKX_SECRETS` given (tests pass it instead of changing the process environment).
pub fn open_for(var: Option<&str>) -> Box<dyn SecretStore> {
    match var {
        Some("memory") => Box::new(MemoryStore::new()),
        Some("none") => Box::new(Unavailable("switched off for this session".into())),
        _ => open_system_store(),
    }
}

/// What a [`Vault::load`] or [`Vault::persist`] pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// Values now in a store.
    pub moved: usize,
    /// Values that no store could take: they live in memory for this session only and are
    /// **never** written to the settings file.
    pub unsaved: usize,
}

/// Where passwords are kept right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The system keyring.
    Keyring,
    /// Bookx's encrypted file, unlocked.
    Vault,
    /// Bookx's encrypted file exists and is locked.
    Locked,
    /// No keyring and no encrypted file yet: a master password can be created.
    Fresh,
    /// Nothing can hold passwords (no keyring, no vault file configured): memory only.
    SessionOnly,
}

/// Something the user should be asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// Passwords were typed or found, and there is no place to keep them: offer a master password.
    Create,
    /// The encrypted file exists: ask for the master password.
    Unlock,
}

/// A [`SecretStore`] (the keyring), an optional encrypted [`FileStore`] used when there is no
/// keyring, and a memory of what is stored, so saving the settings does not talk to the store
/// about values that did not change.
///
/// Rule: a secret is **never** returned into the serialized settings. `persist` blanks every
/// value in the copy it is given, whatever the state of the stores; a value that no store could
/// take stays in the running program's memory only.
pub struct Vault {
    store: Box<dyn SecretStore>,
    file: Option<FileStore>,
    known: Mutex<HashMap<String, String>>,
    declined: AtomicBool,
    attention: AtomicBool,
}

impl Vault {
    pub fn new(store: Box<dyn SecretStore>) -> Vault {
        Vault {
            store,
            file: None,
            known: Mutex::new(HashMap::new()),
            declined: AtomicBool::new(false),
            attention: AtomicBool::new(false),
        }
    }

    /// With an encrypted file as the fallback when the keyring is not there.
    pub fn with_file(store: Box<dyn SecretStore>, file: FileStore) -> Vault {
        Vault {
            file: Some(file),
            ..Vault::new(store)
        }
    }

    /// The store that holds passwords now, if any.
    fn backing(&self) -> Option<&dyn SecretStore> {
        if self.store.available() {
            Some(&*self.store)
        } else {
            self.file
                .as_ref()
                .filter(|f| f.is_unlocked())
                .map(|f| f as &dyn SecretStore)
        }
    }

    pub fn mode(&self) -> Mode {
        if self.store.available() {
            return Mode::Keyring;
        }
        match &self.file {
            None => Mode::SessionOnly,
            Some(f) if f.is_unlocked() => Mode::Vault,
            Some(f) if f.exists() => Mode::Locked,
            Some(_) => Mode::Fresh,
        }
    }

    /// True when passwords are written somewhere safe (keyring or unlocked vault).
    pub fn available(&self) -> bool {
        self.backing().is_some()
    }

    pub fn describe(&self) -> String {
        match self.backing() {
            Some(s) => s.describe(),
            None => "no keyring".into(),
        }
    }

    /// Removes a secret that no setting refers to any more (a deleted destination).
    pub fn forget(&self, key: &str) {
        if let Some(s) = self.backing() {
            if s.delete(key).is_ok() {
                self.known.lock().unwrap().remove(key);
            }
        }
    }

    fn raise_attention(&self) {
        if !self.declined.load(Ordering::Relaxed) {
            self.attention.store(true, Ordering::Relaxed);
        }
    }

    /// The question to ask the user now, once (None when nothing is pending or they said no).
    pub fn take_prompt(&self) -> Option<Prompt> {
        if self.declined.load(Ordering::Relaxed) || !self.attention.swap(false, Ordering::Relaxed) {
            return None;
        }
        match self.mode() {
            Mode::Locked => Some(Prompt::Unlock),
            Mode::Fresh => Some(Prompt::Create),
            _ => None,
        }
    }

    /// "Not now" / "Don't save": no more questions this session; passwords stay in memory.
    pub fn decline(&self) {
        self.declined.store(true, Ordering::Relaxed);
        self.attention.store(false, Ordering::Relaxed);
    }

    pub fn declined(&self) -> bool {
        self.declined.load(Ordering::Relaxed)
    }

    pub fn has_file(&self) -> bool {
        self.file.as_ref().is_some_and(|f| f.exists())
    }

    /// Makes the encrypted store with a master password and opens it. Values that were waiting in
    /// memory are written by the next `persist`.
    pub fn create_vault(&self, passphrase: &str) -> anyhow::Result<()> {
        let f = self
            .file
            .as_ref()
            .ok_or_else(|| anyhow!("no password store is configured"))?;
        f.create(passphrase)?;
        self.known.lock().unwrap().clear();
        Ok(())
    }

    /// Opens the encrypted store. The caller then calls `load` again to fill empty settings.
    pub fn unlock_vault(&self, passphrase: &str) -> Result<(), OpenError> {
        let f = self.file.as_ref().ok_or(OpenError::Missing)?;
        f.unlock(passphrase)?;
        self.known.lock().unwrap().clear();
        Ok(())
    }

    /// Forgets the keys and values of the encrypted store held in memory (it stays on disk).
    pub fn lock_vault(&self) {
        if let Some(f) = &self.file {
            f.lock();
        }
        self.known.lock().unwrap().clear();
    }

    /// Deletes the encrypted file (the master password is lost); stored passwords are gone.
    pub fn reset_vault(&self) -> anyhow::Result<()> {
        if let Some(f) = &self.file {
            f.destroy()?;
        }
        self.known.lock().unwrap().clear();
        Ok(())
    }

    /// After reading the settings file, or after unlocking. A non-empty value is a plain text
    /// secret of an older version (or one typed while locked): it is moved into the store, or kept
    /// in memory when there is none. An empty value is looked up in the store. The caller must
    /// save the settings when `moved + unsaved > 0`: that rewrites the file without them.
    pub fn load(&self, fields: &mut [(String, &mut String)]) -> Migration {
        let mut m = Migration::default();
        let mut known = self.known.lock().unwrap();
        let backing = self.backing();
        for (key, value) in fields.iter_mut() {
            if value.is_empty() {
                if let Some(Ok(Some(v))) = backing.map(|s| s.get(key)) {
                    known.insert(key.clone(), v.clone());
                    **value = v;
                }
            } else if backing.is_some_and(|s| s.set(key, value).is_ok()) {
                known.insert(key.clone(), (**value).clone());
                m.moved += 1;
            } else {
                m.unsaved += 1;
            }
        }
        if m.unsaved > 0 || self.mode() == Mode::Locked {
            self.raise_attention();
        }
        m
    }

    /// Before writing the settings file, on a copy of the settings: stores changed values and
    /// **empties every secret in the copy**, removes the ones the user cleared. A value that no
    /// store could take is only in the running program's memory.
    pub fn persist(&self, fields: &mut [(String, &mut String)]) -> Migration {
        let mut m = Migration::default();
        let mut known = self.known.lock().unwrap();
        let backing = self.backing();
        for (key, value) in fields.iter_mut() {
            if value.is_empty() {
                if known.contains_key(key) && backing.is_some_and(|s| s.delete(key).is_ok()) {
                    known.remove(key);
                }
                continue;
            }
            let stored = known.get(key).is_some_and(|k| k == &**value)
                || backing.is_some_and(|s| s.set(key, value).is_ok());
            if stored {
                known.insert(key.clone(), (**value).clone());
                m.moved += 1;
            } else {
                m.unsaved += 1;
            }
            value.clear();
        }
        if m.unsaved > 0 {
            self.raise_attention();
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn system_store_matches_the_platform() {
        let want = if cfg!(target_os = "macos") {
            "keychain"
        } else if cfg!(windows) {
            "credential-manager"
        } else {
            "secret-service"
        };
        assert_eq!(system_store_name(), want);
    }

    /// Shares a MemoryStore with the test so it can look inside.
    struct Shared(Arc<MemoryStore>);
    impl SecretStore for Shared {
        fn describe(&self) -> String {
            self.0.describe()
        }
        fn get(&self, k: &str) -> Result<Option<String>> {
            self.0.get(k)
        }
        fn set(&self, k: &str, v: &str) -> Result<()> {
            self.0.set(k, v)
        }
        fn delete(&self, k: &str) -> Result<()> {
            self.0.delete(k)
        }
    }

    fn vault() -> (Vault, Arc<MemoryStore>) {
        let m = Arc::new(MemoryStore::new());
        (Vault::new(Box::new(Shared(m.clone()))), m)
    }

    #[test]
    fn memory_store_round_trip() {
        let s = MemoryStore::new();
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "x").unwrap();
        s.set("a", "y").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("y"));
        s.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
    }

    #[test]
    fn loading_moves_plain_text_into_the_store_and_keeps_the_value_in_memory() {
        let (v, m) = vault();
        let (mut a, mut b) = ("pw1".to_string(), String::new());
        let r = v.load(&mut [("one".into(), &mut a), ("two".into(), &mut b)]);
        assert_eq!(
            r,
            Migration {
                moved: 1,
                unsaved: 0
            }
        );
        assert_eq!(a, "pw1");
        assert_eq!(m.get("one").unwrap().as_deref(), Some("pw1"));
    }

    #[test]
    fn loading_fills_empty_values_from_the_store() {
        let (v, m) = vault();
        m.set("one", "from-keyring").unwrap();
        let (mut a, mut b) = (String::new(), "typed".to_string());
        v.load(&mut [("one".into(), &mut a), ("two".into(), &mut b)]);
        assert_eq!(a, "from-keyring");
        assert_eq!(b, "typed");
    }

    #[test]
    fn persisting_empties_the_copy_and_stores_changes_only() {
        let (v, m) = vault();
        let mut a = "pw1".to_string();
        let r = v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(r.moved, 1);
        assert!(a.is_empty(), "the file copy has no secret");
        assert_eq!(m.get("one").unwrap().as_deref(), Some("pw1"));
        // unchanged: the store is not touched again (change it behind the vault's back)
        m.set("one", "other").unwrap();
        let mut a = "pw1".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(m.get("one").unwrap().as_deref(), Some("other"));
        // changed: stored
        let mut a = "pw2".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(m.get("one").unwrap().as_deref(), Some("pw2"));
    }

    #[test]
    fn clearing_a_password_removes_it_from_the_store() {
        let (v, m) = vault();
        let mut a = "pw".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        let mut a = String::new();
        v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(m.get("one").unwrap(), None);
    }

    #[test]
    fn without_a_keyring_nothing_is_ever_written_into_the_copy() {
        let v = Vault::new(Box::new(Unavailable("no daemon".into())));
        assert!(!v.available());
        assert_eq!(v.mode(), Mode::SessionOnly);
        let mut a = "pw1".to_string();
        let r = v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(
            r,
            Migration {
                moved: 0,
                unsaved: 1
            }
        );
        assert!(a.is_empty(), "the file copy has no secret");
        let mut b = "legacy".to_string();
        let r = v.load(&mut [("two".into(), &mut b)]);
        assert_eq!(r.unsaved, 1);
        assert_eq!(b, "legacy", "kept in memory for this session");
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-vault-lib-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn file_vault(d: &std::path::Path) -> Vault {
        Vault::with_file(
            Box::new(Unavailable("none".into())),
            FileStore::with_kdf(d.join("secrets.vault"), Kdf::TEST),
        )
    }

    #[test]
    fn the_encrypted_file_takes_over_when_there_is_no_keyring() {
        let d = tmp("flow");
        let v = file_vault(&d);
        assert_eq!(v.mode(), Mode::Fresh);
        let mut a = "typed-secret".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(v.take_prompt(), Some(Prompt::Create), "asked once");
        assert_eq!(v.take_prompt(), None);
        v.create_vault("a long master password").unwrap();
        assert_eq!(v.mode(), Mode::Vault);
        let mut a = "typed-secret".to_string();
        let r = v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!((r.moved, r.unsaved), (1, 0));
        assert!(a.is_empty());
        let raw = std::fs::read(d.join("secrets.vault")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("typed-secret"));
        // a new session: locked, asks, unlock fills the empty settings
        let v2 = file_vault(&d);
        assert_eq!(v2.mode(), Mode::Locked);
        let mut a = String::new();
        v2.load(&mut [("one".into(), &mut a)]);
        assert!(a.is_empty());
        assert_eq!(v2.take_prompt(), Some(Prompt::Unlock));
        assert!(v2.unlock_vault("wrong wrong wrong").is_err());
        v2.unlock_vault("a long master password").unwrap();
        v2.load(&mut [("one".into(), &mut a)]);
        assert_eq!(a, "typed-secret");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_memory_store_lists_its_keys_sorted_and_never_the_values() {
        let s = MemoryStore::new();
        s.set("b", "secret-b").unwrap();
        s.set("a", "secret-a").unwrap();
        assert_eq!(s.keys(), ["a", "b"]);
        assert_eq!(s.describe(), "a temporary in-memory keyring");
        assert!(s.available());
    }

    #[test]
    fn an_unavailable_store_fails_every_call_with_the_reason() {
        let s = Unavailable("the daemon is gone".into());
        assert!(!s.available());
        assert_eq!(s.describe(), "no keyring");
        for r in [s.get("k").map(|_| ()), s.set("k", "v"), s.delete("k")] {
            assert_eq!(r.unwrap_err().to_string(), "the daemon is gone");
        }
    }

    #[test]
    fn the_keyring_names_its_items_by_application_and_key() {
        assert_eq!(attrs("webdav"), vec![APP_ATTR, ("key", "webdav")]);
        assert_eq!(Oo7Store.describe(), "the system keyring");
    }

    #[test]
    fn the_environment_picks_the_store() {
        let m = open_for(Some("memory"));
        assert!(m.available());
        m.set("k", "v").unwrap();
        assert_eq!(m.get("k").unwrap().as_deref(), Some("v"));
        let n = open_for(Some("none"));
        assert!(!n.available());
        assert_eq!(
            n.get("k").unwrap_err().to_string(),
            "switched off for this session"
        );
    }

    #[test]
    fn forgetting_removes_the_secret_from_the_store_and_the_memory() {
        let (v, m) = vault();
        let mut a = "pw".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(v.mode(), Mode::Keyring);
        assert_eq!(v.describe(), "a temporary in-memory keyring");
        v.forget("one");
        assert_eq!(m.get("one").unwrap(), None);
        // the same value is written again: the vault no longer believes it is stored
        let mut a = "pw".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(m.get("one").unwrap().as_deref(), Some("pw"));
        // nothing to forget without a place to keep passwords
        let none = Vault::new(Box::new(Unavailable("x".into())));
        none.forget("one");
        assert_eq!(none.describe(), "no keyring");
    }

    #[test]
    fn a_working_keyring_never_asks_and_remembers_a_refusal() {
        let (v, _) = vault();
        assert!(!v.declined());
        let mut a = "pw".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(v.take_prompt(), None);
        v.decline();
        assert!(v.declined());
        assert_eq!(v.take_prompt(), None);
    }

    #[test]
    fn a_vault_without_a_file_cannot_make_one_and_has_nothing_to_unlock() {
        let v = Vault::new(Box::new(Unavailable("x".into())));
        assert!(!v.has_file());
        assert!(
            v.create_vault("a long master password")
                .unwrap_err()
                .to_string()
                .contains("no password store")
        );
        assert_eq!(
            v.unlock_vault("a long master password"),
            Err(OpenError::Missing)
        );
        v.lock_vault();
        v.reset_vault().unwrap();
        assert_eq!(v.take_prompt(), None);
    }

    #[test]
    fn locking_and_resetting_the_encrypted_file() {
        let d = tmp("lockreset");
        let v = file_vault(&d);
        v.create_vault("a long master password").unwrap();
        assert!(v.has_file());
        assert!(v.available());
        let mut a = "kept".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        v.lock_vault();
        assert_eq!(v.mode(), Mode::Locked);
        assert!(!v.available());
        assert_eq!(v.describe(), "no keyring");
        // locked: asking is raised by a load with nothing to read, once
        let mut e = String::new();
        v.load(&mut [("one".into(), &mut e)]);
        assert_eq!(v.take_prompt(), Some(Prompt::Unlock));
        // the master password is lost: reset deletes the file, a new one can be made
        v.reset_vault().unwrap();
        assert!(!v.has_file());
        assert_eq!(v.mode(), Mode::Fresh);
        v.create_vault("another long master password").unwrap();
        let mut e = String::new();
        v.load(&mut [("one".into(), &mut e)]);
        assert!(e.is_empty(), "the old passwords are gone");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn declining_means_memory_only_and_no_more_questions() {
        let d = tmp("decline");
        let v = file_vault(&d);
        v.decline();
        let mut a = "typed".to_string();
        let r = v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(r.unsaved, 1);
        assert!(a.is_empty());
        assert_eq!(v.take_prompt(), None);
        assert!(!d.join("secrets.vault").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_locked_store_never_leaks_a_value_into_the_copy_and_keeps_stored_ones() {
        let d = tmp("locked");
        let v = file_vault(&d);
        v.create_vault("a long master password").unwrap();
        let mut a = "kept".to_string();
        v.persist(&mut [("one".into(), &mut a)]);
        let v2 = file_vault(&d);
        let mut b = "typed-while-locked".to_string();
        let r = v2.persist(&mut [("two".into(), &mut b)]);
        assert_eq!(r.unsaved, 1);
        assert!(b.is_empty());
        let mut c = String::new();
        v2.persist(&mut [("one".into(), &mut c)]);
        v2.unlock_vault("a long master password").unwrap();
        let mut c = String::new();
        v2.load(&mut [("one".into(), &mut c)]);
        assert_eq!(
            c, "kept",
            "an empty field while locked did not delete the stored value"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
