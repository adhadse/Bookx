//! Accounts of web services Bookx can read from (Wallabag today; the secrets live in the keyring
//! through `Config::secret_fields`, the rest in the settings file).

use serde::{Deserialize, Serialize};

/// Most imported Wallabag ids remembered (the oldest are forgotten first).
pub const SEEN_LIMIT: usize = 5000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebAccounts {
    pub wallabag_url: String,
    pub wallabag_client_id: String,
    /// Keyring.
    pub wallabag_client_secret: String,
    pub wallabag_user: String,
    /// Keyring.
    pub wallabag_password: String,
    /// An access token pasted by the user instead of the four fields above. Keyring.
    pub wallabag_token: String,
    /// Also take articles that are already archived (read) in Wallabag.
    pub wallabag_include_read: bool,
    /// Only articles with this tag; empty for all.
    pub wallabag_tag: String,
    /// Ids of the articles imported before, so a second import adds only new ones.
    pub wallabag_seen: Vec<u64>,
    /// Hardcover API token (read only lists; no screen yet). Keyring.
    pub hardcover_token: String,
}

impl WebAccounts {
    /// Notes imported ids, keeping the newest [`SEEN_LIMIT`].
    pub fn remember_wallabag(&mut self, ids: &[u64]) {
        for id in ids {
            if !self.wallabag_seen.contains(id) {
                self.wallabag_seen.push(*id);
            }
        }
        let n = self.wallabag_seen.len();
        if n > SEEN_LIMIT {
            self.wallabag_seen.drain(..n - SEEN_LIMIT);
        }
    }

    pub fn wallabag_credentials(&self) -> bookx_sources::wallabag::Credentials {
        bookx_sources::wallabag::Credentials {
            client_id: self.wallabag_client_id.clone(),
            client_secret: self.wallabag_client_secret.clone(),
            username: self.wallabag_user.clone(),
            password: self.wallabag_password.clone(),
        }
    }

    pub fn wallabag_query(&self) -> bookx_sources::wallabag::Query {
        bookx_sources::wallabag::Query {
            unread_only: !self.wallabag_include_read,
            starred_only: false,
            tag: Some(self.wallabag_tag.trim().to_string()).filter(|t| !t.is_empty()),
            limit: 25,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_ids_are_remembered_once_and_bounded() {
        let mut a = WebAccounts::default();
        a.remember_wallabag(&[1, 2, 2, 3]);
        a.remember_wallabag(&[3, 4]);
        assert_eq!(a.wallabag_seen, vec![1, 2, 3, 4]);
        let many: Vec<u64> = (0..SEEN_LIMIT as u64 + 10).collect();
        a.remember_wallabag(&many);
        assert_eq!(a.wallabag_seen.len(), SEEN_LIMIT);
        assert_eq!(*a.wallabag_seen.last().unwrap(), SEEN_LIMIT as u64 + 9);
    }

    #[test]
    fn old_settings_without_accounts_load_and_the_secrets_stay_out_of_the_file() {
        let vault = bookx_secrets::Vault::new(Box::new(bookx_secrets::MemoryStore::new()));
        let (c, _) = crate::config::Config::from_text(Some("{}"), &vault);
        assert_eq!(c.web_accounts, WebAccounts::default());
        let mut c = crate::config::Config::default();
        c.web_accounts.wallabag_url = "https://w.example.org".into();
        c.web_accounts.wallabag_client_secret = "sec-123".into();
        c.web_accounts.wallabag_password = "pw-123".into();
        c.web_accounts.wallabag_token = "tok-123".into();
        c.web_accounts.hardcover_token = "hc-123".into();
        let text = c.file_text(&vault).unwrap();
        for s in ["sec-123", "pw-123", "tok-123", "hc-123"] {
            assert!(!text.contains(s), "{s} must stay in the keyring");
        }
        assert!(text.contains("w.example.org"));
        let (back, _) = crate::config::Config::from_text(Some(&text), &vault);
        assert_eq!(back.web_accounts.wallabag_client_secret, "sec-123");
        assert_eq!(back.web_accounts.wallabag_password, "pw-123");
        assert_eq!(back.web_accounts.wallabag_token, "tok-123");
        assert_eq!(back.web_accounts.hardcover_token, "hc-123");
    }

    #[test]
    fn the_query_follows_the_settings() {
        let mut a = WebAccounts::default();
        assert!(a.wallabag_query().unread_only);
        a.wallabag_include_read = true;
        a.wallabag_tag = " to-read ".into();
        let q = a.wallabag_query();
        assert!(!q.unread_only);
        assert_eq!(q.tag.as_deref(), Some("to-read"));
    }
}
