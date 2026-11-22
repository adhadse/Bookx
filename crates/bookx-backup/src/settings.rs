//! The destinations the user configured, and when each is due.
//!
//! Passwords and the passphrase are plain `String` fields here; the app keeps them out of the
//! settings file (see `bookx-secrets`). They are never printed.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::repo::store::{REPO_DIR, RepoStore};
use crate::repo::{DavStore, FolderStore, Scope};
use crate::target::{FolderTarget, Target, WebDavTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DestKind {
    /// A folder: local, an external drive, a mounted share or a GNOME Online Accounts mount.
    #[default]
    Folder,
    /// A WebDAV folder (Nextcloud, ownCloud, generic).
    Webdav,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Destination {
    pub id: String,
    pub name: String,
    pub kind: DestKind,
    /// The folder path, or the WebDAV address.
    pub location: String,
    pub user: String,
    /// WebDAV password (kept in the keyring by the app).
    pub password: String,
    /// WebDAV: the folder below the address.
    pub folder: String,
    /// How many automatic backups this place keeps (plus one per day for a week; manual ones stay).
    pub keep: usize,
    pub encrypt: bool,
    /// Copy the book files, not just the catalogue (decision D9). On for new and old places.
    pub include_books: bool,
    /// Copy the covers (when books are included, or alone).
    pub include_covers: bool,
    /// Leave out files bigger than this many megabytes; 0 means no limit.
    pub max_file_mb: u64,
    /// The scheduled copies (when Bookx closes, once a day) may use this place. Off until the first
    /// full copy was made by hand (or the user allowed it after seeing how big it is), so an update
    /// never starts a surprise upload of the whole library.
    pub auto_ok: bool,
    pub enabled: bool,
    pub last_ok: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_attempt: Option<DateTime<Utc>>,
}

impl Default for Destination {
    fn default() -> Self {
        Destination {
            id: String::new(),
            name: String::new(),
            kind: DestKind::Folder,
            location: String::new(),
            user: String::new(),
            password: String::new(),
            folder: "Bookx backups".into(),
            keep: 10,
            encrypt: false,
            include_books: true,
            include_covers: true,
            max_file_mb: 0,
            auto_ok: false,
            enabled: true,
            last_ok: None,
            last_error: None,
            last_attempt: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestStatus {
    NeverRun,
    Ok(DateTime<Utc>),
    /// The last attempt failed. `unreachable` when the place itself could not be reached.
    Failed {
        error: String,
        unreachable: bool,
    },
    Off,
}

impl DestStatus {
    pub fn label(&self) -> &'static str {
        match self {
            DestStatus::NeverRun => "Not used yet",
            DestStatus::Ok(_) => "OK",
            DestStatus::Failed {
                unreachable: true, ..
            } => "Unreachable",
            DestStatus::Failed { .. } => "Failed",
            DestStatus::Off => "Switched off",
        }
    }
}

impl Destination {
    pub fn folder(id: &str, name: &str, path: &str) -> Destination {
        Destination {
            id: id.into(),
            name: name.into(),
            kind: DestKind::Folder,
            location: path.into(),
            ..Destination::default()
        }
    }

    pub fn webdav(id: &str, name: &str, url: &str, user: &str, password: &str) -> Destination {
        Destination {
            id: id.into(),
            name: name.into(),
            kind: DestKind::Webdav,
            location: url.into(),
            user: user.into(),
            password: password.into(),
            // off-site by nature
            encrypt: true,
            ..Destination::default()
        }
    }

    pub fn target(&self) -> anyhow::Result<Box<dyn Target>> {
        Ok(match self.kind {
            DestKind::Folder => Box::new(FolderTarget {
                dir: self.location.trim().into(),
            }),
            DestKind::Webdav => Box::new(WebDavTarget::new(
                &self.location,
                &self.user,
                &self.password,
                &self.folder,
            )?),
        })
    }

    /// The place of the incremental repository: the folder [`REPO_DIR`] inside the destination
    /// (next to any old whole-catalogue backups).
    pub fn repo_store(&self) -> anyhow::Result<Box<dyn RepoStore>> {
        Ok(match self.kind {
            DestKind::Folder => Box::new(FolderStore::new(
                std::path::Path::new(self.location.trim()).join(REPO_DIR),
            )),
            DestKind::Webdav => Box::new(DavStore::new(
                &self.location,
                &self.user,
                &self.password,
                &self.folder,
            )?),
        })
    }

    /// What the walk over the library includes for this place.
    pub fn scope(&self) -> Scope {
        Scope {
            include_books: self.include_books,
            include_covers: self.include_covers,
            max_file_size: (self.max_file_mb > 0)
                .then(|| self.max_file_mb.saturating_mul(1024 * 1024)),
        }
    }

    /// A problem with the settings themselves, before anything is tried.
    pub fn problem(&self, passphrase_set: bool) -> Option<String> {
        if self.location.trim().is_empty() {
            return Some("Choose a folder or enter an address".into());
        }
        if self.encrypt && !passphrase_set {
            return Some("Set a backup passphrase to encrypt this destination".into());
        }
        if self.kind == DestKind::Webdav && !self.location.starts_with("http") {
            return Some("A WebDAV address starts with https://".into());
        }
        None
    }

    pub fn status(&self) -> DestStatus {
        if !self.enabled {
            return DestStatus::Off;
        }
        // a failure after the last success is what matters now
        match (&self.last_error, self.last_ok, self.last_attempt) {
            (Some(e), ok, att) if att.is_some_and(|a| ok.is_none_or(|o| a >= o)) => {
                DestStatus::Failed {
                    error: e.clone(),
                    unreachable: looks_unreachable(e),
                }
            }
            (_, Some(ok), _) => DestStatus::Ok(ok),
            _ => DestStatus::NeverRun,
        }
    }

    pub fn record_ok(&mut self, now: DateTime<Utc>) {
        self.last_ok = Some(now);
        self.last_attempt = Some(now);
        self.last_error = None;
    }

    pub fn record_error(&mut self, error: &str, now: DateTime<Utc>) {
        self.last_attempt = Some(now);
        self.last_error = Some(error.to_string());
    }
}

/// Whether an error text means "could not get there" (as opposed to "got there and it went wrong").
pub fn looks_unreachable(e: &str) -> bool {
    let e = e.to_lowercase();
    [
        "could not reach",
        "is not there",
        "plugged in",
        "connecting to",
        "looking up",
        "timed out",
        "took too long",
        "no address",
        "connection",
        "no such file or directory",
    ]
    .iter()
    .any(|k| e.contains(k))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The app is closing.
    Exit,
    /// A periodic look while the app runs.
    Tick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupSettings {
    pub destinations: Vec<Destination>,
    /// Passphrase for destinations with encryption (kept in the keyring by the app).
    pub passphrase: String,
    /// Copy to the destinations when Bookx closes.
    pub on_exit: bool,
    /// And once a day while Bookx is open.
    pub daily: bool,
}

impl Default for BackupSettings {
    fn default() -> Self {
        BackupSettings {
            destinations: Vec::new(),
            passphrase: String::new(),
            on_exit: true,
            daily: true,
        }
    }
}

impl BackupSettings {
    /// A free id for a new destination.
    pub fn new_id(&self) -> String {
        (1..)
            .map(|n| format!("d{n}"))
            .find(|id| !self.destinations.iter().any(|d| &d.id == id))
            .unwrap()
    }

    /// Indices of the destinations to copy to now. A place that failed is retried on the next
    /// exit, and by the timer no more often than every hour.
    pub fn due(&self, trigger: Trigger, now: DateTime<Utc>) -> Vec<usize> {
        let wanted = match trigger {
            Trigger::Exit => self.on_exit,
            Trigger::Tick => self.daily,
        };
        if !wanted {
            return Vec::new();
        }
        self.destinations
            .iter()
            .enumerate()
            .filter(|(_, d)| d.enabled && d.problem(!self.passphrase.is_empty()).is_none())
            .filter(|(_, d)| match trigger {
                Trigger::Exit => d.last_ok.is_none_or(|o| now - o >= Duration::minutes(10)),
                Trigger::Tick => {
                    let day_old = d.last_ok.is_none_or(|o| now - o >= Duration::hours(24));
                    let retry_ok = d.last_attempt.is_none_or(|a| now - a >= Duration::hours(1));
                    day_old && retry_ok
                }
            })
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 12, h, 0, 0).unwrap()
    }

    #[test]
    fn status_follows_the_last_attempt() {
        let mut d = Destination::folder("d1", "NAS", "/mnt/nas");
        assert_eq!(d.status(), DestStatus::NeverRun);
        d.record_ok(t(8));
        assert_eq!(d.status(), DestStatus::Ok(t(8)));
        d.record_error("/mnt/nas is not there. Is the drive plugged in?", t(9));
        match d.status() {
            DestStatus::Failed { unreachable, .. } => assert!(unreachable),
            s => panic!("{s:?}"),
        }
        d.record_error("the cloud folder is full (507)", t(10));
        assert_eq!(d.status().label(), "Failed");
        d.record_ok(t(11));
        assert_eq!(d.status().label(), "OK");
        d.enabled = false;
        assert_eq!(d.status(), DestStatus::Off);
    }

    #[test]
    fn problems_are_found_before_trying() {
        let mut d = Destination::folder("d1", "x", "");
        assert!(d.problem(false).unwrap().contains("Choose"));
        d.location = "/x".into();
        assert!(d.problem(false).is_none());
        d.encrypt = true;
        assert!(d.problem(false).unwrap().contains("passphrase"));
        assert!(d.problem(true).is_none());
        let w = Destination::webdav("d2", "n", "ftp://x", "u", "p");
        assert!(w.problem(true).unwrap().contains("https"));
    }

    #[test]
    fn due_destinations_depend_on_the_trigger_and_the_clock() {
        let mut s = BackupSettings::default();
        s.destinations.push(Destination::folder("d1", "a", "/a"));
        let mut b = Destination::folder("d2", "b", "/b");
        b.record_ok(t(8));
        s.destinations.push(b);
        let mut c = Destination::folder("d3", "c", "/c");
        c.enabled = false;
        s.destinations.push(c);
        let soon = t(8) + Duration::minutes(5);
        assert_eq!(s.due(Trigger::Exit, soon), vec![0]);
        assert_eq!(s.due(Trigger::Tick, soon), vec![0]);
        // a day later b is due for the timer too
        let later = t(8) + Duration::hours(25);
        assert_eq!(s.due(Trigger::Tick, later), vec![0, 1]);
        s.on_exit = false;
        assert!(s.due(Trigger::Exit, later).is_empty());
        s.daily = false;
        assert!(s.due(Trigger::Tick, later).is_empty());
    }

    #[test]
    fn a_failed_place_is_not_hammered_by_the_timer() {
        let mut s = BackupSettings::default();
        let mut d = Destination::folder("d1", "a", "/a");
        d.record_error("could not reach nas", t(9));
        s.destinations.push(d);
        assert!(
            s.due(Trigger::Tick, t(9) + Duration::minutes(20))
                .is_empty()
        );
        assert_eq!(s.due(Trigger::Tick, t(9) + Duration::minutes(61)), vec![0]);
        assert_eq!(s.due(Trigger::Exit, t(9) + Duration::minutes(20)), vec![0]);
    }

    #[test]
    fn encrypted_destinations_wait_for_a_passphrase() {
        let mut s = BackupSettings::default();
        let mut d = Destination::folder("d1", "a", "/a");
        d.encrypt = true;
        s.destinations.push(d);
        assert!(s.due(Trigger::Exit, t(9)).is_empty());
        s.passphrase = "pw".into();
        assert_eq!(s.due(Trigger::Exit, t(9)), vec![0]);
    }

    #[test]
    fn ids_are_unique_and_old_settings_load() {
        let mut s = BackupSettings::default();
        assert_eq!(s.new_id(), "d1");
        s.destinations.push(Destination::folder("d1", "a", "/a"));
        assert_eq!(s.new_id(), "d2");
        let loaded: BackupSettings = serde_json::from_str("{}").unwrap();
        assert!(loaded.on_exit && loaded.destinations.is_empty());
    }

    #[test]
    fn old_destinations_gain_book_backup_and_the_scope_follows_the_settings() {
        let old = r#"{"destinations":[{"id":"d1","name":"USB","kind":"folder","location":"/mnt/usb","keep":4}]}"#;
        let st: BackupSettings = serde_json::from_str(old).unwrap();
        let d = &st.destinations[0];
        assert!(d.include_books && d.include_covers && d.max_file_mb == 0);
        assert!(
            !d.auto_ok,
            "an old place does not start a big upload on its own"
        );
        assert_eq!(d.keep, 4, "old keys still load");
        let sc = d.scope();
        assert!(sc.include_books && sc.include_covers && sc.max_file_size.is_none());
        let mut d = d.clone();
        d.include_books = false;
        d.max_file_mb = 2;
        let sc = d.scope();
        assert!(!sc.include_books);
        assert_eq!(sc.max_file_size, Some(2 * 1024 * 1024));
        assert_eq!(d.repo_store().unwrap().label(), "/mnt/usb/Bookx Backup");
        let w = Destination::webdav("d2", "n", "https://c.example/dav", "u", "p");
        assert!(
            w.repo_store()
                .unwrap()
                .label()
                .ends_with("Bookx backups/Bookx Backup")
        );
    }
}
