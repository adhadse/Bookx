//! Sharing the library over the network (roadmap P4/P10/P12): OPDS catalogue, a small web page
//! and KOReader progress sync, served by `bookx-serve`. No GTK in here, so it is unit tested.

use crate::i18n::tr;
use crate::i18n::trf;
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Result};
use bookx_serve::kosync::{JsonFileStore, KoSync};
use bookx_serve::server::{LibraryProvider, LibrarySnapshot, ServerConfig, User};
use serde::{Deserialize, Serialize};

/// Shortest password accepted when other devices may connect.
pub const MIN_PASSWORD: usize = 6;
/// Name used for the single sharing account.
pub const USER: &str = "bookx";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShareSettings {
    pub port: u16,
    /// Let other devices on the network connect (otherwise only this computer can).
    pub lan: bool,
    pub password: String,
    /// Also serve KOReader's progress-sync endpoints.
    pub kosync: bool,
    /// Also take highlights, notes and bookmarks from the Bookx plugin of KOReader (on by default;
    /// it only matters while the server runs with `kosync`, because it uses the same accounts).
    pub annotations: bool,
    /// While reading, offer to jump when an e-reader reports a position further along (on by
    /// default). Nothing ever moves the page by itself.
    pub offer_jump: bool,
    /// The Bookx plugin was copied to a KOReader device or folder from the Sync page.
    pub plugin_copied: bool,
    /// Serve HTTPS instead of HTTP.
    pub https: bool,
    /// PEM certificate chain for HTTPS; empty together with `key_file` means a self-signed
    /// certificate that Bookx makes (and keeps) itself.
    pub cert_file: String,
    /// PEM private key for HTTPS.
    pub key_file: String,
    /// The server was on when Bookx last ran: it is started again at launch. Set only by the
    /// switches (see [`ShareSettings::remember`]), never by a failed start or by closing the app.
    pub running: bool,
    /// Extra host names the server answers to, besides IP addresses, `localhost` and `.local`
    /// names (the public name behind a reverse proxy). Typed as a list, see
    /// [`bookx_serve::hostguard::parse_host_list`].
    pub allowed_hosts: String,
    /// Reverse proxies (addresses or networks) whose `X-Forwarded-For` is believed by the
    /// failed-login limit; see [`bookx_serve::net::parse_proxy_list`].
    pub trusted_proxies: String,
    /// The sub-path a reverse proxy publishes the server under, such as `/books`.
    pub public_prefix: String,
    /// Extra logins of the library server with the collections each may see (the settings file
    /// holds the Argon2id hash, never the password). No screen edits them yet.
    pub accounts: Vec<SharedAccount>,
    /// Collections (tags) a visitor without a login may read; empty means a login is always needed.
    pub guest_collections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedAccount {
    pub name: String,
    pub hash: String,
    pub collections: Vec<String>,
}

impl Default for ShareSettings {
    fn default() -> Self {
        Self {
            port: 8080,
            lan: false,
            password: String::new(),
            kosync: false,
            annotations: true,
            offer_jump: true,
            plugin_copied: false,
            https: false,
            cert_file: String::new(),
            key_file: String::new(),
            running: false,
            allowed_hosts: String::new(),
            trusted_proxies: String::new(),
            public_prefix: String::new(),
            accounts: Vec::new(),
            guest_collections: Vec::new(),
        }
    }
}

impl ShareSettings {
    /// Why these settings cannot be used, if they cannot.
    pub fn problem(&self) -> Option<String> {
        if self.lan && self.password.chars().count() < MIN_PASSWORD {
            return Some(trf(
                "Set a password of at least {} characters before letting other devices connect.",
                &[&MIN_PASSWORD],
            ));
        }
        if self.lan && self.password.contains(':') {
            return Some(tr("The password must not contain a colon."));
        }
        if self.https && (self.cert_file.trim().is_empty() != self.key_file.trim().is_empty()) {
            return Some(
                tr("Choose both a certificate file and a key file, or neither to use a self-signed certificate."),
            );
        }
        if let Some(p) = self.proxy_problem() {
            return Some(p);
        }
        None
    }

    /// Why the reverse-proxy fields (host names, trusted proxies, sub-path) are not usable.
    pub fn proxy_problem(&self) -> Option<String> {
        if let Err(bad) = bookx_serve::hostguard::parse_host_list(&self.allowed_hosts) {
            return Some(trf(
                "“{}” is not a host name. Use names such as books.example.com, separated by commas.",
                &[&bad],
            ));
        }
        if let Err(bad) = bookx_serve::net::parse_proxy_list(&self.trusted_proxies) {
            return Some(trf(
                "“{}” is not an address of a proxy. Use an IP address or a network such as 10.0.0.0/8.",
                &[&bad],
            ));
        }
        if bookx_serve::server::clean_public_prefix(&self.public_prefix).is_none() {
            return Some(tr(
                "The sub-path must be a plain path such as /books (letters, digits, - and _).",
            ));
        }
        None
    }

    /// Notes what the user asked of the server, to be saved. Switching off is always remembered;
    /// switching on only once it started (`started`), so a busy port keeps the earlier wish and the
    /// next launch tries again. Returns whether the setting changed (the config needs saving).
    pub fn remember(&mut self, on: bool, started: bool) -> bool {
        let want = if on { started || self.running } else { false };
        let changed = self.running != want;
        self.running = want;
        changed
    }

    /// Whether the server is started again at launch.
    pub fn restore_at_launch(&self) -> bool {
        self.running && self.problem().is_none()
    }

    /// True when the certificate comes from files the user chose (otherwise Bookx makes one).
    pub fn own_certificate(&self) -> bool {
        !self.cert_file.trim().is_empty() && !self.key_file.trim().is_empty()
    }

    /// Address to bind: this computer only, or all interfaces.
    pub fn bind(&self) -> &'static str {
        if self.lan {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        }
    }
}

/// A library provider for the server that follows the library database, so books added in the
/// app show up without restarting the server, and big libraries are not re-read per request.
pub fn provider(root: PathBuf) -> LibraryProvider {
    let watcher = bookx_store::CatalogWatcher::new(&root);
    Arc::new(move || LibrarySnapshot {
        root: root.clone(),
        books: watcher.books(),
    })
}

/// Builds the server configuration. `data_dir` holds the KOReader sync store.
pub fn server_config(
    settings: &ShareSettings,
    library: LibraryProvider,
    data_dir: &Path,
) -> Result<ServerConfig> {
    if let Some(p) = settings.problem() {
        bail!(p);
    }
    let mut cfg = ServerConfig::new(library);
    cfg.bind = settings.bind().to_string();
    cfg.port = settings.port;
    cfg.title = tr("Bookx Library");
    cfg.allowed_hosts.extend(
        bookx_serve::hostguard::parse_host_list(&settings.allowed_hosts).unwrap_or_default(),
    );
    cfg.limits.trusted_proxies =
        bookx_serve::net::parse_proxy_list(&settings.trusted_proxies).unwrap_or_default();
    cfg.public_prefix =
        bookx_serve::server::clean_public_prefix(&settings.public_prefix).unwrap_or_default();
    cfg.accounts = settings
        .accounts
        .iter()
        .filter(|a| bookx_serve::keyhash::is_hashed(&a.hash) && !a.name.is_empty())
        .map(|a| bookx_serve::accounts::Account {
            name: a.name.clone(),
            hash: a.hash.clone(),
            collections: a.collections.clone(),
        })
        .collect();
    if !settings.guest_collections.is_empty() {
        cfg.guest = Some(settings.guest_collections.clone());
    }
    if settings.lan || !settings.password.is_empty() {
        cfg.users = vec![User {
            name: USER.into(),
            password: settings.password.clone(),
        }];
    }
    if settings.https {
        cfg.tls = Some(tls_for(settings, data_dir)?);
    }
    if settings.kosync {
        let store = JsonFileStore::open(&data_dir.join("kosync.json"))?;
        let log = Arc::new(bookx_serve::activity::ActivityLog::open(&activity_path(
            data_dir,
        ))?);
        let ko = KoSync::new(Box::new(store), true).with_activity(log.clone());
        ko.on_change(Some(crate::sync_bus::server_listener()));
        cfg.kosync = Some(Arc::new(ko));
        if settings.annotations {
            let ann = bookx_serve::annotation_sync::AnnotationSync::open(
                &data_dir.join("koreader-annotations.json"),
            )?
            .with_activity(log);
            ann.on_change(Some(crate::sync_bus::server_listener()));
            cfg.annotations = Some(Arc::new(ann));
        }
    }
    Ok(cfg)
}

/// The file of the sync activity (devices that connected and what they sent).
pub fn activity_path(data_dir: &Path) -> PathBuf {
    data_dir.join("sync-activity.json")
}

/// Folder inside the data folder where the self-signed certificate is kept.
pub fn tls_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("tls")
}

/// Names the self-signed certificate is valid for: this computer's names and, when other devices
/// may connect, its LAN address.
fn certificate_names(settings: &ShareSettings) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(h) = std::env::var("HOSTNAME") {
        names.push(h);
    }
    if settings.lan {
        if let Some(ip) = lan_address() {
            names.push(ip.to_string());
        }
    }
    names
}

/// Makes a new self-signed certificate and key in the data folder (replacing an earlier one).
/// Returns the paths of the two files.
pub fn generate_certificate(
    settings: &ShareSettings,
    data_dir: &Path,
) -> Result<(PathBuf, PathBuf)> {
    bookx_serve::cert::write_self_signed(&tls_dir(data_dir), &certificate_names(settings))
}

/// The TLS configuration for `settings`: the chosen files, else the self-signed pair in the data
/// folder (made on first use, kept afterwards so clients that trusted it keep working).
pub fn tls_for(
    settings: &ShareSettings,
    data_dir: &Path,
) -> Result<Arc<bookx_serve::server::TlsConfig>> {
    if settings.own_certificate() {
        let (c, k) = (
            PathBuf::from(settings.cert_file.trim()),
            PathBuf::from(settings.key_file.trim()),
        );
        return bookx_serve::net::tls_config(&c, &k)
            .map_err(|e| anyhow::anyhow!("The certificate cannot be used: {e:#}"));
    }
    let dir = tls_dir(data_dir);
    let (c, k) = (
        dir.join(bookx_serve::cert::CERT_FILE),
        dir.join(bookx_serve::cert::KEY_FILE),
    );
    if c.is_file() && k.is_file() {
        if let Ok(cfg) = bookx_serve::net::tls_config(&c, &k) {
            return Ok(cfg);
        }
    }
    let (c, k) = generate_certificate(settings, data_dir)?;
    bookx_serve::net::tls_config(&c, &k)
}

/// The running server, if any.
#[derive(Default)]
pub struct Sharing {
    handle: Option<bookx_serve::server::ServerHandle>,
    annotations: Option<Arc<bookx_serve::annotation_sync::AnnotationSync>>,
    activity: Option<Arc<bookx_serve::activity::ActivityLog>>,
    settings: Option<ShareSettings>,
}

impl Sharing {
    pub fn running(&self) -> bool {
        self.handle.is_some()
    }

    /// Starts (or restarts with new settings) the server for the library in `root`. Returns the
    /// addresses to show.
    pub fn start(
        &mut self,
        settings: &ShareSettings,
        root: &Path,
        data_dir: &Path,
    ) -> Result<Vec<String>> {
        self.stop();
        let cfg = server_config(settings, provider(root.to_path_buf()), data_dir)?;
        let annotations = cfg.annotations.clone();
        let activity = cfg.kosync.as_ref().and_then(|k| k.activity());
        let handle = bookx_serve::server::Server::start(cfg).map_err(|e| {
            anyhow::anyhow!("Could not start sharing on port {}: {e:#}", settings.port)
        })?;
        let urls = urls(settings, handle.addr().port());
        self.handle = Some(handle);
        self.annotations = annotations;
        self.activity = activity;
        self.settings = Some(settings.clone());
        Ok(urls)
    }

    pub fn stop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.shutdown();
        }
        self.settings = None;
        self.annotations = None;
        self.activity = None;
    }

    /// The KOReader annotation store of the running server, when it serves one.
    pub fn annotations(&self) -> Option<Arc<bookx_serve::annotation_sync::AnnotationSync>> {
        self.annotations.clone()
    }

    /// The activity log of the running server (devices and what they sent), when it is on.
    pub fn activity(&self) -> Option<Arc<bookx_serve::activity::ActivityLog>> {
        self.activity.clone()
    }

    /// Addresses of the running server.
    pub fn urls(&self) -> Vec<String> {
        match (&self.handle, &self.settings) {
            (Some(h), Some(s)) => urls(s, h.addr().port()),
            _ => Vec::new(),
        }
    }
}

/// The address to type into KOReader's custom sync server (and the Bookx plugin), from the
/// addresses of the running server (the last one is the one on the local network when there is
/// one). It ends in `/bookx`: reading progress (KOSync protocol) and annotations share it; the
/// server has no other mount.
pub fn kosync_url(urls: &[String]) -> Option<String> {
    urls.last().map(|u| {
        format!(
            "{}{}",
            u.trim_end_matches('/'),
            bookx_serve::server::SYNC_PREFIX
        )
    })
}

/// Addresses to show the user: always this computer, plus this machine's LAN address when
/// other devices may connect.
pub fn urls(settings: &ShareSettings, actual_port: u16) -> Vec<String> {
    let scheme = if settings.https { "https" } else { "http" };
    let mut v = vec![format!("{scheme}://127.0.0.1:{actual_port}/")];
    if settings.lan {
        if let Some(ip) = lan_address() {
            v.push(format!("{scheme}://{ip}:{actual_port}/"));
        }
    }
    v
}

/// The address of the interface used for outgoing traffic. Nothing is sent: connecting a UDP
/// socket only asks the OS which route it would take.
fn lan_address() -> Option<std::net::IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[cfg(test)]
mod tests {
    #[test]
    fn offering_to_jump_is_on_by_default_and_for_a_settings_file_that_has_not_heard_of_it() {
        assert!(ShareSettings::default().offer_jump);
        let old: ShareSettings = serde_json::from_str(r#"{"port":8080,"kosync":true}"#).unwrap();
        assert!(old.offer_jump);
        let off: ShareSettings = serde_json::from_str(r#"{"offer_jump":false}"#).unwrap();
        assert!(!off.offer_jump);
    }

    #[test]
    fn the_server_wish_survives_a_failed_start_and_is_cleared_only_by_switching_off() {
        let mut s = super::ShareSettings::default();
        assert!(!s.restore_at_launch());
        assert!(s.remember(true, true), "on and started: remembered");
        assert!(s.restore_at_launch());
        assert!(!s.remember(true, false), "a failed restart keeps the wish");
        assert!(s.running);
        assert!(s.remember(false, true), "switching off forgets it");
        assert!(!s.restore_at_launch());
        assert!(
            !s.remember(true, false),
            "a first start that fails is not remembered"
        );
        assert!(!s.running);
    }

    #[test]
    fn a_server_that_cannot_start_with_these_settings_is_not_restored() {
        let s = super::ShareSettings {
            running: true,
            lan: true,
            ..Default::default()
        };
        assert!(!s.restore_at_launch(), "network access without a password");
    }

    use super::*;

    #[test]
    fn the_koreader_address_and_explanation() {
        assert_eq!(kosync_url(&[]), None);
        let urls = vec![
            "http://127.0.0.1:8080/".to_string(),
            "http://192.168.1.5:8080/".to_string(),
        ];
        assert_eq!(
            kosync_url(&urls).as_deref(),
            Some("http://192.168.1.5:8080/bookx")
        );
        assert_eq!(
            kosync_url(&urls[..1]).as_deref(),
            Some("http://127.0.0.1:8080/bookx")
        );
    }

    use bookx_serve::server::Server;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-share-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn library_with_book(root: &Path, title: &str) {
        let md = crate::core::opf::Metadata {
            title: title.into(),
            authors: vec!["Ann".into()],
            ..Default::default()
        };
        let ch = crate::core::convert::Chapter {
            title: "One".into(),
            body: "<p>text</p>".into(),
        };
        let epub = root.join(format!("{title}.epub"));
        std::fs::write(
            &epub,
            crate::core::convert::create_epub(&md, &[ch], None)
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        let mut lib = bookx_store::open_library(&root.join("lib"))
            .unwrap()
            .library;
        lib.add_path(&epub, true).unwrap();
        lib.save().unwrap();
    }

    fn get(addr: &str, path: &str, auth: Option<&str>) -> (u16, String) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let a = auth
            .map(|a| format!("Authorization: Basic {a}\r\n"))
            .unwrap_or_default();
        // a server that answers early (and closes) may reset the connection under load
        let _ = write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: x\r\n{a}Connection: close\r\n\r\n"
        );
        let mut buf = String::new();
        let _ = s.read_to_string(&mut buf);
        let code = buf
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        (code, buf)
    }

    #[test]
    fn accounts_and_guest_reach_the_server_config_and_plain_passwords_do_not() {
        let lib: bookx_serve::server::LibraryProvider =
            std::sync::Arc::new(bookx_serve::server::LibrarySnapshot::default);
        let good = bookx_serve::accounts::Account::new("kid", "pw", &["kids".into()]);
        let s = ShareSettings {
            accounts: vec![
                SharedAccount {
                    name: good.name.clone(),
                    hash: good.hash.clone(),
                    collections: good.collections.clone(),
                },
                // a hand-written plain password in the file is not a login
                SharedAccount {
                    name: "plain".into(),
                    hash: "secret".into(),
                    collections: vec![],
                },
            ],
            guest_collections: vec!["kids".into()],
            ..ShareSettings::default()
        };
        let cfg = server_config(&s, lib, std::path::Path::new("/nonexistent")).unwrap();
        assert_eq!(cfg.accounts.len(), 1);
        assert_eq!(cfg.guest.as_deref(), Some(&["kids".to_string()][..]));
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("\"pw\""));
    }

    #[test]
    fn defaults_are_private() {
        let s = ShareSettings::default();
        assert!(!s.lan && !s.kosync);
        assert_eq!(s.bind(), "127.0.0.1");
        assert!(s.problem().is_none());
    }

    /// Highlights, notes and bookmarks sync is on by default (deliberate: it is what the Bookx
    /// plugin needs), and it is independent of the server: nothing listens until the server is
    /// started, and with Bookx sync on the annotation store is there.
    #[test]
    fn annotation_sync_is_on_by_default_and_independent_of_the_server() {
        let s = ShareSettings::default();
        assert!(s.annotations, "the switch is on by default");
        assert!(!s.kosync && !s.lan, "but nothing is shared by default");
        assert!(
            !Sharing::default().running(),
            "nothing listens until the server is started"
        );
        // an old config without the key gets the default; an explicit choice is kept
        let old: ShareSettings = serde_json::from_str(r#"{"port": 9000}"#).unwrap();
        assert!(old.annotations);
        let off: ShareSettings = serde_json::from_str(r#"{"annotations": false}"#).unwrap();
        assert!(!off.annotations);
        let dir = tmp("annotdefault");
        let prov = provider(dir.join("lib"));
        // server without Bookx sync: no accounts, so no annotation store
        let plain = server_config(&s, prov.clone(), &dir).unwrap();
        assert!(plain.annotations.is_none() && plain.kosync.is_none());
        // with Bookx sync on, annotation sync is available without touching the switch
        let sync = ShareSettings {
            kosync: true,
            ..Default::default()
        };
        let c = server_config(&sync, prov, &dir).unwrap();
        assert!(c.kosync.is_some() && c.annotations.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_configs_without_sharing_settings_load() {
        let s: ShareSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, ShareSettings::default());
        let s: ShareSettings = serde_json::from_str(r#"{"port": 9000}"#).unwrap();
        assert_eq!(s.port, 9000);
    }

    #[test]
    fn letting_the_network_in_needs_a_proper_password() {
        let mut s = ShareSettings {
            lan: true,
            ..Default::default()
        };
        assert!(s.problem().unwrap().contains("password"));
        s.password = "short".into();
        assert!(s.problem().is_some());
        s.password = "six-ch".into();
        assert!(s.problem().is_none());
        assert_eq!(s.bind(), "0.0.0.0");
        s.password = "has:colon".into();
        assert!(s.problem().unwrap().contains("colon"));
        // a short password is fine while only this computer can connect
        s.lan = false;
        s.password = "x".into();
        assert!(s.problem().is_none());
    }

    #[test]
    fn config_refuses_bad_settings_and_sets_up_the_account() {
        let dir = tmp("cfg");
        let prov = provider(dir.join("lib"));
        let bad = ShareSettings {
            lan: true,
            ..Default::default()
        };
        assert!(server_config(&bad, prov.clone(), &dir).is_err());

        let local = server_config(&ShareSettings::default(), prov.clone(), &dir).unwrap();
        assert!(local.users.is_empty());
        assert_eq!(local.bind, "127.0.0.1");
        assert!(local.kosync.is_none());

        let lan = ShareSettings {
            lan: true,
            password: "secret1".into(),
            kosync: true,
            port: 9191,
            ..Default::default()
        };
        let c = server_config(&lan, prov, &dir).unwrap();
        assert_eq!(c.bind, "0.0.0.0");
        assert_eq!(c.port, 9191);
        assert_eq!(c.users.len(), 1);
        assert_eq!(c.users[0].name, USER);
        assert!(c.kosync.is_some());
        assert!(dir.join("kosync.json").exists() || c.kosync.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn provider_follows_the_catalogue_on_disk() {
        let dir = tmp("prov");
        library_with_book(&dir, "First");
        let prov = provider(dir.join("lib"));
        assert_eq!(prov().books.len(), 1);
        // unchanged file: served from the cache
        assert_eq!(prov().books.len(), 1);
        // a book added later shows up without restarting anything
        let mut lib = bookx_store::open_library(&dir.join("lib")).unwrap().library;
        let md = crate::core::opf::Metadata {
            title: "Second".into(),
            authors: vec!["Bob".into()],
            ..Default::default()
        };
        let ch = crate::core::convert::Chapter {
            title: "One".into(),
            body: "<p>more text</p>".into(),
        };
        let p = dir.join("second.epub");
        std::fs::write(
            &p,
            crate::core::convert::create_epub(&md, &[ch], None)
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        lib.add_path(&p, true).unwrap();
        lib.save().unwrap();
        assert_eq!(prov().books.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn provider_of_a_missing_library_is_empty_not_a_crash() {
        let dir = tmp("missing");
        let prov = provider(dir.join("nope"));
        let snap = prov();
        assert!(snap.books.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn urls_always_include_localhost_and_the_lan_only_when_shared() {
        let local = urls(&ShareSettings::default(), 8123);
        assert_eq!(local, vec!["http://127.0.0.1:8123/"]);
        let lan = ShareSettings {
            lan: true,
            password: "secret1".into(),
            ..Default::default()
        };
        let v = urls(&lan, 8123);
        assert_eq!(v[0], "http://127.0.0.1:8123/");
        assert!(v.iter().all(|u| u.ends_with(":8123/")));
    }

    #[test]
    fn the_controller_starts_restarts_and_stops() {
        let dir = tmp("ctl");
        library_with_book(&dir, "Ctl");
        let root = dir.join("lib");
        let mut sh = Sharing::default();
        assert!(!sh.running() && sh.urls().is_empty());
        let settings = ShareSettings {
            port: 0,
            ..Default::default()
        };
        let urls = sh.start(&settings, &root, &dir).unwrap();
        assert!(sh.running());
        assert_eq!(sh.urls(), urls);
        let addr = urls[0]
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();
        assert_eq!(get(&addr, "/", None).0, 200);
        // restarting replaces the server
        let again = sh.start(&settings, &root, &dir).unwrap();
        assert!(sh.running());
        assert_eq!(again.len(), 1);
        let addr2 = again[0]
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();
        assert_eq!(
            get(&addr2, "/", None).0,
            200,
            "the restarted server answers"
        );
        sh.stop();
        assert!(!sh.running() && sh.urls().is_empty());
        assert!(
            TcpStream::connect(&addr).is_err(),
            "port still open after stop"
        );
        // stopping twice is harmless
        sh.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_controller_reports_a_busy_port_and_bad_settings() {
        let dir = tmp("busy");
        let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let mut sh = Sharing::default();
        let err = sh
            .start(
                &ShareSettings {
                    port,
                    ..Default::default()
                },
                &dir.join("lib"),
                &dir,
            )
            .unwrap_err();
        assert!(format!("{err:#}").contains(&port.to_string()), "{err:#}");
        assert!(!sh.running());
        let err = sh
            .start(
                &ShareSettings {
                    lan: true,
                    ..Default::default()
                },
                &dir.join("lib"),
                &dir,
            )
            .unwrap_err();
        assert!(format!("{err:#}").contains("password"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_real_server_serves_the_library_and_enforces_the_password() {
        let dir = tmp("serve");
        library_with_book(&dir, "Served");
        let settings = ShareSettings {
            port: 0,
            password: "secret1".into(),
            ..Default::default()
        };
        let cfg = server_config(&settings, provider(dir.join("lib")), &dir).unwrap();
        let h = Server::start(cfg).unwrap();
        let addr = h.addr().to_string();
        assert_eq!(get(&addr, "/opds", None).0, 401);
        // base64("bookx:secret1")
        let ok = "Ym9va3g6c2VjcmV0MQ==";
        let (code, body) = get(&addr, "/", Some(ok));
        assert_eq!(code, 200);
        assert!(body.contains("Served"), "{body}");
        assert_eq!(get(&addr, "/opds", Some(ok)).0, 200);
        h.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn https_settings_are_checked_and_old_files_still_load() {
        let s: ShareSettings = serde_json::from_str(r#"{"port": 9000}"#).unwrap();
        assert!(!s.https && !s.own_certificate());
        let mut s = ShareSettings {
            https: true,
            cert_file: "/x/c.pem".into(),
            ..Default::default()
        };
        assert!(s
            .problem()
            .unwrap()
            .contains("both a certificate file and a key"));
        s.key_file = "/x/k.pem".into();
        assert!(s.problem().is_none() && s.own_certificate());
        s.cert_file.clear();
        s.key_file.clear();
        assert!(
            s.problem().is_none(),
            "no files means a self-signed certificate"
        );
        assert_eq!(urls(&s, 8443)[0], "https://127.0.0.1:8443/");
        s.https = false;
        assert_eq!(urls(&s, 8080)[0], "http://127.0.0.1:8080/");
    }

    #[test]
    fn the_self_signed_certificate_is_made_once_and_kept() {
        let d = tmp("tlsgen");
        let s = ShareSettings {
            https: true,
            ..Default::default()
        };
        tls_for(&s, &d).unwrap();
        let cert = tls_dir(&d).join(bookx_serve::cert::CERT_FILE);
        let first = std::fs::read(&cert).unwrap();
        tls_for(&s, &d).unwrap();
        assert_eq!(std::fs::read(&cert).unwrap(), first, "reused, not remade");
        generate_certificate(&s, &d).unwrap();
        assert_ne!(
            std::fs::read(&cert).unwrap(),
            first,
            "regenerating replaces it"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn chosen_certificate_files_must_work() {
        let d = tmp("tlsfiles");
        let mut s = ShareSettings {
            https: true,
            cert_file: d.join("no.pem").display().to_string(),
            key_file: d.join("no.key").display().to_string(),
            ..Default::default()
        };
        let e = tls_for(&s, &d).unwrap_err().to_string();
        assert!(e.contains("cannot be used"), "{e}");
        let (c, k) = bookx_serve::cert::write_self_signed(&d.join("own"), &[]).unwrap();
        s.cert_file = c.display().to_string();
        s.key_file = k.display().to_string();
        assert!(tls_for(&s, &d).is_ok());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_real_tls_client_gets_the_catalogue_from_a_started_share() {
        use rustls::pki_types::{pem::PemObject, CertificateDer, ServerName};
        let d = tmp("tlsrun");
        let root = tmp("tlsrun-lib");
        library_with_book(&root, "Over Tls");
        let s = ShareSettings {
            port: 0,
            https: true,
            ..Default::default()
        };
        let mut sh = Sharing::default();
        let urls = sh.start(&s, &root.join("lib"), &d).unwrap();
        assert!(urls[0].starts_with("https://127.0.0.1:"), "{urls:?}");
        let port: u16 = urls[0]
            .trim_end_matches('/')
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let pem = std::fs::read(tls_dir(&d).join(bookx_serve::cert::CERT_FILE)).unwrap();
        let cert = CertificateDer::from_pem_slice(&pem).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let conn = rustls::ClientConnection::new(
            cfg,
            ServerName::try_from("localhost".to_string()).unwrap(),
        )
        .unwrap();
        let mut tls = rustls::StreamOwned::new(conn, tcp);
        write!(
            tls,
            "GET /opds HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut body = Vec::new();
        let _ = tls.read_to_end(&mut body);
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(text.contains("<feed"), "{text}");
        // plain HTTP to the same port gets no catalogue
        let (code, plain) = get(&format!("127.0.0.1:{port}"), "/opds", None);
        assert!(code != 200 && !plain.contains("<feed"), "{plain}");
        sh.stop();
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::remove_dir_all(root);
    }
}
