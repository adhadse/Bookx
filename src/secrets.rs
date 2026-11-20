//! Passwords in the system keyring (Secret Service / Flatpak portal) instead of the settings file.
//! The mechanics are in `bookx-secrets` and `Config::{from_text, file_text}`; this file owns the one
//! vault of the process and the words the dialogs use.
//!
//! `BOOKX_SECRETS=memory` (the UI tests, and every unit test of this crate) keeps secrets in
//! memory only; `BOOKX_SECRETS=none` pretends there is no keyring.

use std::sync::OnceLock;

use bookx_secrets::{MemoryStore, SecretStore, Vault};

static VAULT: OnceLock<Vault> = OnceLock::new();

/// The process-wide vault. Connecting to the keyring happens on first use.
pub fn vault() -> &'static Vault {
    VAULT.get_or_init(|| {
        let store: Box<dyn SecretStore> = if cfg!(test) {
            Box::new(MemoryStore::new())
        } else {
            bookx_secrets::open_default()
        };
        Vault::new(store)
    })
}

/// The caption under a password field: where the password is kept.
pub fn caption(available: bool) -> &'static str {
    if available {
        crate::i18n::N_("Kept in the system keyring, not in the settings file.")
    } else {
        crate::i18n::N_("No keyring was found, so this is kept in the settings file (unencrypted). Install GNOME Keyring or KeePassXC to protect it.")
    }
}

/// [`caption`] for this session.
pub fn caption_now() -> String {
    crate::i18n::tr(caption(vault().available()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caption_says_where_the_password_is_and_warns_without_a_keyring() {
        assert!(caption(true).contains("keyring") && !caption(true).contains("unencrypted"));
        assert!(caption(false).contains("settings file (unencrypted)"));
    }

    #[test]
    fn tests_never_touch_the_real_keyring() {
        assert!(vault().available());
        assert_eq!(vault().describe(), "a temporary in-memory keyring");
    }
}
