//! Site accounts for news and web downloads: a user name and password for a paywalled site, sent as
//! HTTP Basic authentication to **that host only**.
//!
//! * The password lives in the keyring through `Config::secret_fields` (key `news-site-<host>`), never
//!   in the settings file; the file keeps the host and the user name.
//! * The header is added only for an `https` address whose host is exactly the account's host (no
//!   sub-domains, no other port-less matches), so a feed that redirects elsewhere never gets it:
//!   the HTTP client does not forward it on redirects (ureq's default), and a plain `http` address
//!   gets nothing because the password would travel in the clear.
//! * Every request still goes through the guarded fetcher (the SSRF guard), so a site account cannot
//!   point Bookx at a private address.
//!
//! No GTK in here, so it is unit tested; the screen is part of `ui/news_recipes.rs`.

use std::sync::Arc;

use base64::Engine;
use bookx_sources::fetch::Fetcher;
use serde::{Deserialize, Serialize};

use crate::i18n::tr;

/// Most site accounts.
pub const MAX_SITES: usize = 40;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteAccount {
    /// Lower-case host name, for example `news.example.com`.
    pub host: String,
    pub user: String,
    /// Keyring (see `Config::secret_fields`); empty in the settings file.
    pub password: String,
}

/// The keyring key of the password of `host`.
pub fn secret_key(host: &str) -> String {
    format!("news-site-{host}")
}

/// The host a person typed or pasted (a host name or a whole address), or why it is not usable.
pub fn normalize_host(input: &str) -> Result<String, String> {
    let t = input.trim();
    if t.is_empty() {
        return Err(tr("Type the address of the site."));
    }
    let with_scheme = if t.contains("://") {
        t.to_string()
    } else {
        format!("https://{t}")
    };
    if t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(tr("The address must not contain spaces."));
    }
    let url = url::Url::parse(&with_scheme).map_err(|_| tr("This is not a site address."))?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err(tr("Only http and https sites can be used."));
    }
    let host = bookx_sources::netguard::normalize_host(url.host_str().unwrap_or(""));
    if host.is_empty() || !host.contains('.') && host != "localhost" {
        return Err(tr("This is not a site address."));
    }
    Ok(host)
}

/// Why a site account cannot be saved, if it cannot.
pub fn check(
    host_input: &str,
    user: &str,
    password: &str,
    existing: &[SiteAccount],
    editing: Option<&str>,
) -> Result<SiteAccount, String> {
    let host = normalize_host(host_input)?;
    let user = user.trim();
    if user.is_empty() {
        return Err(tr("Type the user name."));
    }
    if user.contains(':') || user.chars().any(|c| c.is_control()) {
        // Basic authentication joins the two with a colon
        return Err(tr("The user name must not contain a colon."));
    }
    if password.is_empty() {
        return Err(tr("Type the password."));
    }
    if existing
        .iter()
        .any(|a| a.host == host && Some(a.host.as_str()) != editing)
    {
        return Err(tr("This site has an account already."));
    }
    if editing.is_none() && existing.len() >= MAX_SITES {
        return Err(tr("No more site accounts can be added."));
    }
    Ok(SiteAccount {
        host,
        user: user.to_string(),
        password: password.to_string(),
    })
}

/// The value of the `Authorization` header.
pub fn basic_header(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

/// What the screen says about an account (never the password).
pub fn summary(a: &SiteAccount) -> String {
    crate::i18n::trf("User: {}", &[&a.user])
}

/// Adds the header of the matching site account to the requests of another fetcher.
pub struct SiteAuth {
    inner: Arc<dyn Fetcher>,
    accounts: Vec<SiteAccount>,
}

impl SiteAuth {
    pub fn new(inner: Arc<dyn Fetcher>, accounts: &[SiteAccount]) -> Self {
        SiteAuth {
            inner,
            accounts: accounts
                .iter()
                .filter(|a| !a.host.is_empty() && !a.user.is_empty() && !a.password.is_empty())
                .cloned()
                .collect(),
        }
    }

    fn headers_for(&self, url: &str, headers: &[(String, String)]) -> Vec<(String, String)> {
        let mut h = headers.to_vec();
        let secure = url
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("https://");
        let host = bookx_sources::netguard::host_of(url);
        let has_auth = h
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
        if let (true, Some(host), false) = (secure, host, has_auth) {
            if let Some(a) = self.accounts.iter().find(|a| a.host == host) {
                h.push((
                    "Authorization".to_string(),
                    basic_header(&a.user, &a.password),
                ));
            }
        }
        h
    }
}

impl Fetcher for SiteAuth {
    fn get(&self, url: &str, headers: &[(String, String)]) -> anyhow::Result<Vec<u8>> {
        self.inner.get(url, &self.headers_for(url, headers))
    }

    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        content_type: &str,
        body: &[u8],
    ) -> anyhow::Result<Vec<u8>> {
        self.inner
            .post(url, &self.headers_for(url, headers), content_type, body)
    }
}

/// The fetcher of the web downloads with the site accounts on top (without accounts: the plain one).
pub fn fetcher_with(accounts: &[SiteAccount]) -> Arc<dyn Fetcher> {
    let base = crate::web_import::fetcher();
    if accounts.is_empty() {
        base
    } else {
        Arc::new(SiteAuth::new(base, accounts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_sources::fetch::MockFetcher;

    fn acc(host: &str) -> SiteAccount {
        SiteAccount {
            host: host.into(),
            user: "reader".into(),
            password: "sentinel-site-pw".into(),
        }
    }

    #[test]
    fn hosts_are_taken_from_names_and_addresses() {
        assert_eq!(
            normalize_host("News.Example.com").unwrap(),
            "news.example.com"
        );
        assert_eq!(
            normalize_host("https://news.example.com/feed.xml?x=1").unwrap(),
            "news.example.com"
        );
        assert_eq!(normalize_host("example.com:8443/a").unwrap(), "example.com");
        assert!(normalize_host("").is_err());
        assert!(normalize_host("ftp://example.com").is_err());
        assert!(normalize_host("two words.com").is_err());
        assert!(normalize_host("intranet").is_err());
    }

    #[test]
    fn an_account_needs_a_user_a_password_and_a_new_host() {
        let have = vec![acc("a.example.com")];
        assert!(check("b.example.com", "me", "pw", &have, None).is_ok());
        assert!(check("b.example.com", "", "pw", &have, None).is_err());
        assert!(check("b.example.com", "me", "", &have, None).is_err());
        assert!(check("b.example.com", "m:e", "pw", &have, None).is_err());
        assert!(check("A.example.com", "me", "pw", &have, None).is_err());
        assert!(check("a.example.com", "me", "pw", &have, Some("a.example.com")).is_ok());
        let many: Vec<SiteAccount> = (0..MAX_SITES)
            .map(|i| acc(&format!("s{i}.example.com")))
            .collect();
        assert!(check("new.example.com", "me", "pw", &many, None).is_err());
    }

    #[test]
    fn the_header_goes_to_that_host_over_https_only() {
        let mock = Arc::new(
            MockFetcher::new()
                .with_body("https://news.example.com/feed", "x")
                .with_body("https://other.example.com/feed", "x")
                .with_body("http://news.example.com/feed", "x")
                .with_body("https://sub.news.example.com/feed", "x"),
        );
        let f = SiteAuth::new(mock.clone(), &[acc("news.example.com")]);
        for u in [
            "https://news.example.com/feed",
            "https://other.example.com/feed",
            "http://news.example.com/feed",
            "https://sub.news.example.com/feed",
        ] {
            f.get(u, &[]).unwrap();
        }
        let r = mock.requests();
        assert_eq!(
            r[0].header_value("authorization"),
            Some(basic_header("reader", "sentinel-site-pw").as_str())
        );
        assert!(r[1].header_value("authorization").is_none(), "another host");
        assert!(r[2].header_value("authorization").is_none(), "plain http");
        assert!(r[3].header_value("authorization").is_none(), "a sub-domain");
    }

    #[test]
    fn a_header_the_caller_set_is_not_replaced_and_an_empty_account_is_ignored() {
        let mock = Arc::new(MockFetcher::new().with_body("https://news.example.com/a", "x"));
        let f = SiteAuth::new(mock.clone(), &[acc("news.example.com")]);
        f.get(
            "https://news.example.com/a",
            &[("Authorization".into(), "Bearer t".into())],
        )
        .unwrap();
        assert_eq!(
            mock.requests()[0].header_value("authorization"),
            Some("Bearer t")
        );
        let mut empty = acc("news.example.com");
        empty.password.clear();
        let mock = Arc::new(MockFetcher::new().with_body("https://news.example.com/a", "x"));
        SiteAuth::new(mock.clone(), &[empty])
            .get("https://news.example.com/a", &[])
            .unwrap();
        assert!(mock.requests()[0].header_value("authorization").is_none());
    }

    #[test]
    fn the_basic_header_is_the_standard_one() {
        // RFC 7617 example
        assert_eq!(
            basic_header("Aladdin", "open sesame"),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn the_password_is_a_secret_field_and_stays_out_of_the_file() {
        let mut cfg = crate::config::Config::default();
        cfg.news_sites.push(acc("news.example.com"));
        let keys: Vec<String> = cfg.secret_fields().into_iter().map(|(k, _)| k).collect();
        assert!(keys.contains(&"news-site-news.example.com".to_string()));
        let vault = bookx_secrets::Vault::new(Box::new(bookx_secrets::MemoryStore::new()));
        let text = cfg.file_text(&vault).unwrap();
        assert!(!text.contains("sentinel-site-pw"));
        assert!(text.contains("news.example.com") && text.contains("reader"));
    }
}
