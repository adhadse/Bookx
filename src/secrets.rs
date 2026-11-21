//! Passwords in the system keyring (Secret Service / Flatpak portal) or, without one, in Bookx's own
//! encrypted password store (`secrets.vault`, master password), never in the settings file.
//! The mechanics are in `bookx-secrets` and `Config::{from_text, file_text}`; this file owns the one
//! vault of the process and the words the dialogs use.
//!
//! `BOOKX_SECRETS=memory` (the UI tests, and every unit test of this crate) keeps secrets in
//! memory only; `none` pretends there is no keyring (so the encrypted store and the "not saved"
//! paths are used).

use std::path::PathBuf;
use std::sync::OnceLock;

use bookx_secrets::{FileStore, MemoryStore, Mode, SecretStore, Vault};

static VAULT: OnceLock<Vault> = OnceLock::new();

/// Where the encrypted password store lives: next to the settings file.
pub fn vault_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("secrets.vault"))
}

/// The process-wide vault. Connecting to the keyring happens on first use.
pub fn vault() -> &'static Vault {
    VAULT.get_or_init(|| {
        if cfg!(test) {
            return Vault::new(Box::new(MemoryStore::new()));
        }
        let store: Box<dyn SecretStore> = bookx_secrets::open_default();
        match vault_path() {
            Some(p) if !std::env::var("BOOKX_SECRETS").is_ok_and(|v| v == "memory") => {
                Vault::with_file(store, FileStore::new(p))
            }
            _ => Vault::new(store),
        }
    })
}

/// The caption under a password field: where the password is kept.
pub fn caption(mode: Mode) -> &'static str {
    match mode {
        Mode::Keyring => crate::i18n::N_("Kept in the system keyring, not in the settings file."),
        Mode::Vault => crate::i18n::N_("Kept in Bookx's encrypted password store."),
        Mode::Locked | Mode::Fresh | Mode::SessionOnly => {
            crate::i18n::N_("Not saved: it is forgotten when Bookx closes.")
        }
    }
}

/// [`caption`] for this session.
pub fn caption_now() -> String {
    crate::i18n::tr(caption(vault().mode()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caption_says_where_the_password_is_and_never_claims_the_settings_file() {
        assert!(caption(Mode::Keyring).contains("keyring"));
        assert!(caption(Mode::Vault).contains("encrypted password store"));
        for m in [Mode::Locked, Mode::Fresh, Mode::SessionOnly] {
            assert!(caption(m).contains("Not saved"));
        }
        for m in [
            Mode::Keyring,
            Mode::Vault,
            Mode::Locked,
            Mode::Fresh,
            Mode::SessionOnly,
        ] {
            assert!(!caption(m).contains("unencrypted"), "{m:?}");
        }
    }

    #[test]
    fn tests_never_touch_the_real_keyring() {
        assert!(vault().available());
        assert_eq!(vault().describe(), "a temporary in-memory keyring");
    }
}
