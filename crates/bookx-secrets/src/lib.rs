#![forbid(unsafe_code)]
//! Passwords in the Secret Service instead of the settings file.
//!
//! **Why `oo7`?** It speaks the freedesktop Secret Service API (GNOME Keyring, KWallet, KeePassXC)
//! and, inside a Flatpak, the `org.freedesktop.portal.Secrets` portal with an encrypted file
//! keyring, with no C library (`libsecret`) in the build. It is the library GNOME's own apps use.
//! The async runtime is the small `async-io` one that `zbus` already brings; nothing here needs
//! tokio.
//!
//! Everything goes through the [`SecretStore`] trait, so the logic around it (migrating plain text
//! values, falling back, caching) is tested with [`MemoryStore`] and no desktop session.
//!
//! Secrets are never written to a log: no function here prints a value, and the error texts
//! carry only keys and D-Bus messages.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{anyhow, Result};

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

const APP_ATTR: (&str, &str) = ("application", "com.adhadse.Bookx");

/// The Secret Service (or the Flatpak portal) through `oo7`.
pub struct Oo7Store;

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

fn attrs(key: &str) -> Vec<(&str, &str)> {
    vec![APP_ATTR, ("key", key)]
}

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

/// The store to use: `BOOKX_SECRETS=memory` (tests), `none` (pretend there is no keyring), else
/// the Secret Service, and when it does not answer, [`Unavailable`].
pub fn open_default() -> Box<dyn SecretStore> {
    match std::env::var("BOOKX_SECRETS").as_deref() {
        Ok("memory") => Box::new(MemoryStore::new()),
        Ok("none") => Box::new(Unavailable("switched off for this session".into())),
        _ => match Oo7Store::connect() {
            Ok(s) => Box::new(s),
            Err(e) => Box::new(Unavailable(format!("{e:#}"))),
        },
    }
}

/// What a [`Vault::load`] or [`Vault::persist`] pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// Values now in the store.
    pub moved: usize,
    /// Values that could not be stored and stay in the settings file.
    pub kept: usize,
}

/// A [`SecretStore`] plus a memory of what it holds, so saving the settings does not talk to the
/// keyring about values that did not change.
pub struct Vault {
    store: Box<dyn SecretStore>,
    known: Mutex<HashMap<String, String>>,
}

impl Vault {
    pub fn new(store: Box<dyn SecretStore>) -> Vault {
        Vault {
            store,
            known: Mutex::new(HashMap::new()),
        }
    }

    pub fn available(&self) -> bool {
        self.store.available()
    }

    pub fn describe(&self) -> String {
        self.store.describe()
    }

    /// Removes a secret that no setting refers to any more (a deleted destination).
    pub fn forget(&self, key: &str) {
        if self.store.delete(key).is_ok() {
            self.known.lock().unwrap().remove(key);
        }
    }

    /// After reading the settings file. A non-empty value in the file is a plain text secret from
    /// an older version: it is stored (and the caller should save the settings, which then leave
    /// it out of the file). An empty value is looked up in the store. Values stay in memory.
    pub fn load(&self, fields: &mut [(String, &mut String)]) -> Migration {
        let mut m = Migration::default();
        let mut known = self.known.lock().unwrap();
        for (key, value) in fields.iter_mut() {
            if value.is_empty() {
                if let Ok(Some(v)) = self.store.get(key) {
                    known.insert(key.clone(), v.clone());
                    **value = v;
                }
            } else if self.store.set(key, value).is_ok() {
                known.insert(key.clone(), (**value).clone());
                m.moved += 1;
            } else {
                m.kept += 1;
            }
        }
        m
    }

    /// Before writing the settings file, on a copy of the settings: stores changed values and
    /// empties them in the copy, removes the ones the user cleared. A value that cannot be stored
    /// stays in the copy, so it is not lost.
    pub fn persist(&self, fields: &mut [(String, &mut String)]) -> Migration {
        let mut m = Migration::default();
        let mut known = self.known.lock().unwrap();
        for (key, value) in fields.iter_mut() {
            if value.is_empty() {
                if known.contains_key(key) && self.store.delete(key).is_ok() {
                    known.remove(key);
                }
                continue;
            }
            if known.get(key).is_some_and(|k| k == &**value) || self.store.set(key, value).is_ok() {
                known.insert(key.clone(), (**value).clone());
                value.clear();
                m.moved += 1;
            } else {
                m.kept += 1;
            }
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        assert_eq!(r, Migration { moved: 1, kept: 0 });
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
    fn without_a_keyring_the_values_stay_in_the_file() {
        let v = Vault::new(Box::new(Unavailable("no daemon".into())));
        assert!(!v.available());
        let mut a = "pw1".to_string();
        let r = v.persist(&mut [("one".into(), &mut a)]);
        assert_eq!(r, Migration { moved: 0, kept: 1 });
        assert_eq!(a, "pw1");
        let mut b = "legacy".to_string();
        let r = v.load(&mut [("two".into(), &mut b)]);
        assert_eq!(r.kept, 1);
        assert_eq!(b, "legacy");
    }
}
