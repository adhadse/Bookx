//! Places a backup can be written to. A [`Target`] is a folder-like thing with a handful of
//! operations; the folder target also covers everything the desktop shows as a folder: external
//! drives, SMB/NFS shares and GNOME Online Accounts (Google Drive, OneDrive, Nextcloud) mounted by
//! GVfs under `/run/user/<uid>/gvfs`.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use bookx_serve::webdav::WebDav;
use chrono::{DateTime, Utc};

use bookx_store::health::{parse_snapshot_name, SnapshotKind};

/// Largest backup file read back from a WebDAV server.
pub const MAX_DOWNLOAD: usize = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub name: String,
    pub size: u64,
}

pub trait Target: Send + Sync {
    /// "/mnt/nas/bookx" or "https://cloud.example.com/…".
    fn label(&self) -> String;
    /// Reaches the place and can write there. The error says what is wrong in plain words.
    fn check(&self) -> Result<()>;
    /// Backup files (names accepted by [`parse_remote_name`]) in the place.
    fn list(&self) -> Result<Vec<RemoteFile>>;
    fn put(&self, name: &str, data: &[u8]) -> Result<()>;
    fn get(&self, name: &str) -> Result<Vec<u8>>;
    fn delete(&self, name: &str) -> Result<()>;
    /// Free bytes, when the place can tell.
    fn free_space(&self) -> Option<u64> {
        None
    }
}

/// What a backup file name says: when, how many books, what kind, encrypted or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteName {
    pub at: DateTime<Utc>,
    pub books: usize,
    pub kind: SnapshotKind,
    pub encrypted: bool,
}

/// `bookx-20261112T101500Z-n123-auto.db` (plain) or `….bookxbackup` (encrypted).
pub fn parse_remote_name(name: &str) -> Option<RemoteName> {
    let (stem, encrypted) = match name.strip_suffix(".bookxbackup") {
        Some(s) => (s, true),
        None => (name.strip_suffix(".db")?, false),
    };
    let (at, books, kind) = parse_snapshot_name(&format!("{stem}.db"))?;
    Some(RemoteName {
        at,
        books,
        kind,
        encrypted,
    })
}

/// The name a snapshot gets in a place: `.db` for plain copies, `.bookxbackup` for encrypted.
pub fn remote_name(snapshot_file_name: &str, encrypted: bool) -> String {
    if encrypted {
        format!(
            "{}.bookxbackup",
            snapshot_file_name
                .strip_suffix(".db")
                .unwrap_or(snapshot_file_name)
        )
    } else {
        snapshot_file_name.to_string()
    }
}

fn check_name(name: &str) -> Result<()> {
    if parse_remote_name(name).is_none() || name.contains(['/', '\\', '\0']) {
        bail!("{name:?} is not a Bookx backup file name");
    }
    Ok(())
}

// -- a folder ---------------------------------------------------------------------------------------

pub struct FolderTarget {
    pub dir: PathBuf,
}

impl Target for FolderTarget {
    fn label(&self) -> String {
        self.dir.display().to_string()
    }

    fn check(&self) -> Result<()> {
        if !self.dir.exists() {
            // the parent has to exist (a drive that is not plugged in must not be "created")
            let parent = self.dir.parent().filter(|p| !p.as_os_str().is_empty());
            if parent.is_some_and(|p| !p.is_dir()) {
                bail!(
                    "{} is not there. Is the drive plugged in or the share mounted?",
                    self.dir.display()
                );
            }
        } else if !self.dir.is_dir() {
            bail!("{} is not a folder", self.dir.display());
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("cannot create {}", self.dir.display()))?;
        let probe = self.dir.join(".bookx-write-test");
        std::fs::write(&probe, b"x")
            .with_context(|| format!("cannot write in {}", self.dir.display()))?;
        let _ = std::fs::remove_file(&probe);
        Ok(())
    }

    fn list(&self) -> Result<Vec<RemoteFile>> {
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(anyhow!("cannot read {}: {e}", self.dir.display())),
        };
        Ok(rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                parse_remote_name(&name)?;
                Some(RemoteFile {
                    name,
                    size: e.metadata().map(|m| m.len()).unwrap_or(0),
                })
            })
            .collect())
    }

    fn put(&self, name: &str, data: &[u8]) -> Result<()> {
        check_name(name)?;
        std::fs::create_dir_all(&self.dir)?;
        let part = self.dir.join(format!(".{name}.part"));
        let dest = self.dir.join(name);
        std::fs::write(&part, data).with_context(|| format!("writing {}", part.display()))?;
        if std::fs::rename(&part, &dest).is_err() {
            // some mounts (FUSE) refuse to rename: write the file directly
            std::fs::write(&dest, data).with_context(|| format!("writing {}", dest.display()))?;
            let _ = std::fs::remove_file(&part);
        }
        Ok(())
    }

    fn get(&self, name: &str) -> Result<Vec<u8>> {
        check_name(name)?;
        std::fs::read(self.dir.join(name)).with_context(|| format!("reading {name}"))
    }

    fn delete(&self, name: &str) -> Result<()> {
        check_name(name)?;
        match std::fs::remove_file(self.dir.join(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow!("cannot delete {name}: {e}")),
        }
    }

    fn free_space(&self) -> Option<u64> {
        bookx_serve::devfs::free_space(&self.dir).map(|s| s.free)
    }
}

// -- WebDAV -------------------------------------------------------------------------------------------

pub struct WebDavTarget {
    dav: WebDav,
    folder: Vec<String>,
}

impl WebDavTarget {
    /// `folder` is a path below the server address, `/` separated ("Bookx backups").
    pub fn new(url: &str, user: &str, password: &str, folder: &str) -> Result<WebDavTarget> {
        let folder: Vec<String> = folder
            .split('/')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        if folder.iter().any(|p| p == ".." || p == ".") {
            bail!("the folder name must not contain .. or .");
        }
        Ok(WebDavTarget {
            dav: WebDav::new(url, user, password)?,
            folder,
        })
    }
}

impl Target for WebDavTarget {
    fn label(&self) -> String {
        format!("{}{}", self.dav.base_url(), self.folder.join("/"))
    }

    fn check(&self) -> Result<()> {
        self.dav.check()?;
        self.dav.ensure_dirs(&self.folder)?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<RemoteFile>> {
        Ok(self
            .dav
            .list_dir(&self.folder)?
            .into_iter()
            .filter(|e| !e.is_dir)
            .filter_map(|e| {
                let name = e.rel.rsplit('/').next()?.to_string();
                parse_remote_name(&name)?;
                Some(RemoteFile { name, size: e.size })
            })
            .collect())
    }

    fn put(&self, name: &str, data: &[u8]) -> Result<()> {
        check_name(name)?;
        self.dav.put_replace(&self.folder, name, data)
    }

    fn get(&self, name: &str) -> Result<Vec<u8>> {
        check_name(name)?;
        self.dav.get_file(&self.folder, name, MAX_DOWNLOAD)
    }

    fn delete(&self, name: &str) -> Result<()> {
        check_name(name)?;
        self.dav.delete_file(&self.folder, name)
    }

    fn free_space(&self) -> Option<u64> {
        self.dav.quota().map(|q| q.free)
    }
}

// -- in memory (tests) ----------------------------------------------------------------------------------

/// A target that lives in memory, can be made unreachable, and can corrupt what it stores.
#[derive(Default)]
pub struct MemTarget {
    pub files: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    pub down: Mutex<Option<String>>,
    /// Flip a byte of everything written (to test verify-after-write).
    pub corrupt: Mutex<bool>,
}

impl MemTarget {
    pub fn set_down(&self, why: Option<&str>) {
        *self.down.lock().unwrap() = why.map(str::to_string);
    }
    fn up(&self) -> Result<()> {
        match &*self.down.lock().unwrap() {
            Some(why) => Err(anyhow!("{why}")),
            None => Ok(()),
        }
    }
}

impl Target for MemTarget {
    fn label(&self) -> String {
        "memory".into()
    }
    fn check(&self) -> Result<()> {
        self.up()
    }
    fn list(&self) -> Result<Vec<RemoteFile>> {
        self.up()?;
        Ok(self
            .files
            .lock()
            .unwrap()
            .iter()
            .map(|(n, d)| RemoteFile {
                name: n.clone(),
                size: d.len() as u64,
            })
            .collect())
    }
    fn put(&self, name: &str, data: &[u8]) -> Result<()> {
        self.up()?;
        check_name(name)?;
        let mut d = data.to_vec();
        if *self.corrupt.lock().unwrap() {
            if let Some(b) = d.last_mut() {
                *b ^= 0xff;
            }
        }
        self.files.lock().unwrap().insert(name.to_string(), d);
        Ok(())
    }
    fn get(&self, name: &str) -> Result<Vec<u8>> {
        self.up()?;
        self.files
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("{name} is not there"))
    }
    fn delete(&self, name: &str) -> Result<()> {
        self.up()?;
        self.files.lock().unwrap().remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_for_both_kinds() {
        let n = "bookx-20261112T101500Z-n123-auto.db";
        let r = parse_remote_name(n).unwrap();
        assert!(!r.encrypted && r.books == 123 && r.kind == SnapshotKind::Auto);
        let e = remote_name(n, true);
        assert_eq!(e, "bookx-20261112T101500Z-n123-auto.bookxbackup");
        let r = parse_remote_name(&e).unwrap();
        assert!(r.encrypted && r.books == 123);
        assert_eq!(remote_name(n, false), n);
        assert!(parse_remote_name("holiday.db").is_none());
        assert!(parse_remote_name("../bookx-20261112T101500Z-n1-auto.db").is_none());
    }

    #[test]
    fn a_folder_target_writes_lists_reads_and_deletes_only_backups() {
        let d = std::env::temp_dir().join(format!("bookx-bk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let t = FolderTarget { dir: d.join("sub") };
        t.check().unwrap();
        let name = "bookx-20261112T101500Z-n3-manual.db";
        t.put(name, b"abc").unwrap();
        std::fs::write(d.join("sub/notes.txt"), "mine").unwrap();
        let l = t.list().unwrap();
        assert_eq!(
            l,
            vec![RemoteFile {
                name: name.into(),
                size: 3
            }]
        );
        assert_eq!(t.get(name).unwrap(), b"abc");
        assert!(t.delete("notes.txt").is_err(), "only backup names");
        assert!(t.delete("../x.db").is_err());
        t.delete(name).unwrap();
        assert!(t.list().unwrap().is_empty());
        assert!(d.join("sub/notes.txt").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_missing_drive_is_not_created_out_of_thin_air() {
        let t = FolderTarget {
            dir: PathBuf::from("/definitely-not-mounted/backups"),
        };
        let e = t.check().unwrap_err().to_string();
        assert!(e.contains("plugged in") || e.contains("mounted"), "{e}");
    }
}
