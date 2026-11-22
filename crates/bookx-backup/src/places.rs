//! Telling the user what kind of place a folder is, and finding the places worth offering:
//! external drives, network shares and the cloud drives GNOME Online Accounts mounts.
//!
//! **How cloud drives work here.** Bookx does not talk to Google Drive or OneDrive. GNOME Online
//! Accounts (Settings → Online Accounts) mounts them through GVfs, and the desktop exposes the
//! mount as a folder under `/run/user/<uid>/gvfs/`. A backup is then an ordinary file copy into
//! that folder. Nextcloud and ownCloud can be used the same way, or with the built-in WebDAV
//! destination. iCloud has no public API or GVfs backend; the only route is a folder that a
//! desktop sync client keeps in step (see [`icloud_candidates`]).

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

/// What kind of place a folder is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceKind {
    /// A folder on this computer's own disk.
    Local,
    /// A removable or external drive.
    External,
    /// A network share (SMB, NFS, SFTP, FTP, AFP).
    Network(String),
    /// A cloud drive mounted by GNOME Online Accounts.
    Cloud(String),
    /// A WebDAV mount (Nextcloud, ownCloud).
    WebDav,
    /// A folder an iCloud desktop client syncs.
    ICloud,
}

impl PlaceKind {
    pub fn label(&self) -> String {
        match self {
            PlaceKind::Local => "This computer".into(),
            PlaceKind::External => "External drive".into(),
            PlaceKind::Network(p) => format!("Network share ({p})"),
            PlaceKind::Cloud(n) => n.clone(),
            PlaceKind::WebDav => "WebDAV (Nextcloud, ownCloud…)".into(),
            PlaceKind::ICloud => "iCloud Drive folder".into(),
        }
    }

    /// Whether a copy here leaves the computer (a real off-site copy).
    pub fn off_site(&self) -> bool {
        !matches!(self, PlaceKind::Local | PlaceKind::External)
    }
}

/// One mount the user could back up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: PathBuf,
    pub name: String,
    pub kind: PlaceKind,
}

/// Kind and a readable name of a GVfs mount directory name such as
/// `smb-share:server=nas,share=books` or `google-drive:host=gmail.com,user=me%40gmail.com`.
pub fn gvfs_mount(dir_name: &str) -> Option<(PlaceKind, String)> {
    let (scheme, rest) = dir_name.split_once(':')?;
    let get = |k: &str| -> Option<String> {
        rest.split(',').find_map(|kv| {
            let (key, v) = kv.split_once('=')?;
            (key == k).then(|| percent_decode_str(v).decode_utf8_lossy().into_owned())
        })
    };
    let host = get("server").or_else(|| get("host")).unwrap_or_default();
    let user = get("user").unwrap_or_default();
    Some(match scheme {
        "smb-share" => (
            PlaceKind::Network("SMB".into()),
            format!("{} on {host}", get("share").unwrap_or_default()),
        ),
        "nfs" => (PlaceKind::Network("NFS".into()), format!("NFS on {host}")),
        "sftp" => (PlaceKind::Network("SFTP".into()), format!("SFTP on {host}")),
        "ftp" => (PlaceKind::Network("FTP".into()), format!("FTP on {host}")),
        "afp-volume" => (
            PlaceKind::Network("AFP".into()),
            format!("{} on {host}", get("volume").unwrap_or_default()),
        ),
        "google-drive" => (PlaceKind::Cloud("Google Drive".into()), user),
        "onedrive" => (PlaceKind::Cloud("OneDrive".into()), user),
        "dav" | "davs" => (PlaceKind::WebDav, format!("{user} on {host}")),
        _ => return None,
    })
}

/// Everything GVfs has mounted in `gvfs_dir` (`/run/user/<uid>/gvfs`).
pub fn gvfs_places(gvfs_dir: &Path) -> Vec<Place> {
    let mut out: Vec<Place> = std::fs::read_dir(gvfs_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let (kind, name) = gvfs_mount(&n)?;
            Some(Place {
                path: e.path(),
                name: if name.trim().is_empty() {
                    kind.label()
                } else {
                    name
                },
                kind,
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn unescape_mount_path(s: &str) -> String {
    // /proc/mounts writes spaces as \040
    s.replace("\\040", " ").replace("\\011", "\t")
}

/// Mounts from the text of `/proc/mounts`: network file systems anywhere, and anything under the
/// places desktops put removable drives (`/run/media/<user>`, `/media/<user>`, `/mnt`).
pub fn mount_places(proc_mounts: &str) -> Vec<Place> {
    let mut out = Vec::new();
    for line in proc_mounts.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        let (dev, mp, fs) = (f[0], unescape_mount_path(f[1]), f[2]);
        let net = match fs {
            "nfs" | "nfs4" => Some("NFS"),
            "cifs" | "smb3" | "smbfs" => Some("SMB"),
            "sshfs" | "fuse.sshfs" => Some("SFTP"),
            _ => None,
        };
        let name = Path::new(&mp)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| mp.clone());
        if let Some(proto) = net {
            out.push(Place {
                path: PathBuf::from(&mp),
                name: format!("{name} ({dev})"),
                kind: PlaceKind::Network(proto.into()),
            });
        } else if (mp.starts_with("/run/media/")
            || mp.starts_with("/media/")
            || mp.starts_with("/mnt/"))
            && dev.starts_with("/dev/")
        {
            out.push(Place {
                path: PathBuf::from(&mp),
                name,
                kind: PlaceKind::External,
            });
        }
    }
    out
}

/// Folders where a desktop iCloud client usually keeps the Drive, if they exist.
pub fn icloud_candidates(home: &Path) -> Vec<Place> {
    ["iCloud Drive", "iCloudDrive", "iCloud"]
        .iter()
        .map(|n| home.join(n))
        .filter(|p| p.is_dir())
        .map(|path| Place {
            path,
            name: "iCloud Drive (synced by a desktop client)".into(),
            kind: PlaceKind::ICloud,
        })
        .collect()
}

/// All places that can be found on this computer.
pub fn detect_places() -> Vec<Place> {
    let mut out = Vec::new();
    if let Some(dir) = gvfs_dir() {
        out.extend(gvfs_places(&dir));
    }
    if let Ok(t) = std::fs::read_to_string("/proc/mounts") {
        out.extend(mount_places(&t));
    }
    if let Some(h) = std::env::var_os("HOME") {
        out.extend(icloud_candidates(Path::new(&h)));
    }
    out
}

/// `/run/user/<uid>/gvfs`, or the same under `XDG_RUNTIME_DIR`.
pub fn gvfs_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")?;
    let p = Path::new(&base).join("gvfs");
    p.is_dir().then_some(p)
}

/// What kind of place `path` is, judged by where it lives. `places` is the list from
/// [`detect_places`] (passed in so this stays testable).
pub fn classify(path: &Path, places: &[Place]) -> PlaceKind {
    // the most specific mount wins
    let best = places
        .iter()
        .filter(|p| path.starts_with(&p.path))
        .max_by_key(|p| p.path.components().count());
    if let Some(p) = best {
        return p.kind.clone();
    }
    let s = path.to_string_lossy();
    if s.contains("/gvfs/") {
        if let Some(name) = s
            .rsplit("/gvfs/")
            .next()
            .and_then(|r| r.split('/').next())
            .and_then(gvfs_mount)
        {
            return name.0;
        }
    }
    PlaceKind::Local
}

/// The sentence that tells the user how to get a cloud drive in the list.
pub const CLOUD_HELP: &str = "Google Drive, OneDrive and Nextcloud: add the account in GNOME Settings → Online Accounts and switch on “Files”. It then appears below as a mounted place. iCloud has no way in from Linux except a folder that a desktop sync client keeps up to date.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gvfs_mount_names_are_understood() {
        let (k, n) = gvfs_mount("smb-share:server=nas.local,share=books").unwrap();
        assert_eq!(k, PlaceKind::Network("SMB".into()));
        assert_eq!(n, "books on nas.local");
        let (k, n) = gvfs_mount("google-drive:host=gmail.com,user=anna%40gmail.com").unwrap();
        assert_eq!(k, PlaceKind::Cloud("Google Drive".into()));
        assert_eq!(n, "anna@gmail.com");
        assert_eq!(
            gvfs_mount("onedrive:host=outlook.com,user=a%40outlook.com")
                .unwrap()
                .0,
            PlaceKind::Cloud("OneDrive".into())
        );
        let (k, n) = gvfs_mount(
            "dav:host=cloud.example.org,ssl=true,user=bob,prefix=%2Fremote.php%2Fwebdav",
        )
        .unwrap();
        assert_eq!(k, PlaceKind::WebDav);
        assert_eq!(n, "bob on cloud.example.org");
        assert!(gvfs_mount("mtp:host=Phone").is_none());
        assert!(gvfs_mount("nonsense").is_none());
    }

    #[test]
    fn a_gvfs_folder_lists_only_the_mounts_that_hold_files() {
        let d = std::env::temp_dir().join(format!("bookx-gvfs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join("smb-share:server=nas,share=media")).unwrap();
        std::fs::create_dir_all(d.join("google-drive:host=gmail.com,user=a%40gmail.com")).unwrap();
        std::fs::create_dir_all(d.join("mtp:host=phone")).unwrap();
        let p = gvfs_places(&d);
        assert_eq!(p.len(), 2);
        assert!(
            p.iter()
                .any(|x| x.kind == PlaceKind::Cloud("Google Drive".into()))
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn proc_mounts_gives_drives_and_network_shares() {
        let t = "/dev/sda1 / ext4 rw 0 0\n/dev/sdb1 /run/media/anna/BACKUP ext4 rw 0 0\nsrv:/export /mnt/nas nfs4 rw 0 0\n//srv/share /mnt/my\\040share cifs rw 0 0\ntmpfs /tmp tmpfs rw 0 0\n";
        let p = mount_places(t);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].kind, PlaceKind::External);
        assert_eq!(p[0].name, "BACKUP");
        assert_eq!(p[1].kind, PlaceKind::Network("NFS".into()));
        assert_eq!(p[2].path, PathBuf::from("/mnt/my share"));
    }

    #[test]
    fn classify_uses_the_most_specific_mount_and_knows_off_site() {
        let places = mount_places(
            "//srv/share /mnt/nas cifs rw 0 0\n/dev/sdb1 /run/media/a/D ext4 rw 0 0\n",
        );
        assert_eq!(
            classify(Path::new("/mnt/nas/bookx"), &places),
            PlaceKind::Network("SMB".into())
        );
        assert_eq!(
            classify(Path::new("/run/media/a/D/x"), &places),
            PlaceKind::External
        );
        assert_eq!(
            classify(Path::new("/home/a/backups"), &places),
            PlaceKind::Local
        );
        assert_eq!(
            classify(
                Path::new("/run/user/1000/gvfs/google-drive:host=gmail.com,user=a/My Drive"),
                &[]
            ),
            PlaceKind::Cloud("Google Drive".into())
        );
        assert!(!PlaceKind::Local.off_site());
        assert!(!PlaceKind::External.off_site());
        assert!(PlaceKind::WebDav.off_site());
    }
}
