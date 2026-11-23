//! Protecting a library from the two ways SQLite libraries get hurt: two writers at once, and a
//! folder on a network or synced file system (roadmap PP-C-3, PP-G-13).
//!
//! * [`acquire`] takes a lock file `<library>/.bookx-lock` naming the process that uses the
//!   library. A second Bookx gets [`Lock::Busy`] with a message it can show, and opens the library
//!   read-only. A lock left behind by a crashed process on this machine is taken over.
//! * [`fs_warnings`] reports a library on NFS/SMB/sshfs-like mounts or inside a Dropbox, Syncthing,
//!   Nextcloud … folder.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The lock file inside a library folder.
pub const LOCK_FILE: &str = ".bookx-lock";

/// Who holds a library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockHolder {
    pub pid: u32,
    pub host: String,
    /// Unix seconds.
    pub since: i64,
    pub app: String,
}

impl LockHolder {
    /// A sentence for the user.
    pub fn describe(&self) -> String {
        let when = chrono::DateTime::from_timestamp(self.since, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_default();
        format!(
            "This library is already open in {} (process {} on {}, since {when}). \
             Close it there first, or open this library read-only. If that program is gone, \
             delete the file {LOCK_FILE} in the library folder.",
            self.app, self.pid, self.host
        )
    }
}

/// The result of asking for the lock.
#[derive(Debug)]
pub enum Lock {
    /// This process owns the library until the guard is dropped.
    Acquired(LibraryLock),
    /// Another live process owns it.
    Busy(LockHolder),
}

/// Removes the lock file when dropped, if it still names this process.
#[derive(Debug)]
pub struct LibraryLock {
    path: PathBuf,
    pid: u32,
}

impl Drop for LibraryLock {
    fn drop(&mut self) {
        if read_holder(&self.path).is_some_and(|h| h.pid == self.pid) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn read_holder(path: &Path) -> Option<LockHolder> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The name of this machine.
pub fn host_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "this computer".into())
}

/// Whether a process with this id exists on this machine (Linux; elsewhere assumed alive).
pub fn pid_alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        Path::new(&format!("/proc/{pid}")).exists()
    } else {
        true
    }
}

/// Takes the lock of the library in `root` for the current process.
pub fn acquire(root: &Path, app: &str) -> Result<Lock> {
    acquire_with(root, app, std::process::id(), &host_name(), &pid_alive)
}

/// [`acquire`] with the process id, host name and liveness check given (tests).
pub fn acquire_with(
    root: &Path,
    app: &str,
    pid: u32,
    host: &str,
    alive: &dyn Fn(u32) -> bool,
) -> Result<Lock> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("cannot create the library folder {}", root.display()))?;
    let path = root.join(LOCK_FILE);
    let mine = LockHolder {
        pid,
        host: host.to_string(),
        since: chrono::Utc::now().timestamp(),
        app: app.to_string(),
    };
    for _ in 0..3 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(&serde_json::to_vec(&mine)?)
                    .with_context(|| format!("cannot write {}", path.display()))?;
                return Ok(Lock::Acquired(LibraryLock { path, pid }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match read_holder(&path) {
                    // a lock of a dead process on this machine, or an unreadable one, is stale
                    Some(h) if h.host == host && h.pid != pid && !alive(h.pid) => {
                        let _ = std::fs::remove_file(&path);
                    }
                    None => {
                        let _ = std::fs::remove_file(&path);
                    }
                    Some(h) => return Ok(Lock::Busy(h)),
                }
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!(
                        "cannot take the lock {} (is the folder read-only?)",
                        path.display()
                    )
                });
            }
        }
    }
    // somebody keeps racing us for it: report whoever has it now
    match read_holder(&path) {
        Some(h) => Ok(Lock::Busy(h)),
        None => anyhow::bail!("cannot take the lock {}", path.display()),
    }
}

// -- opening a library with the guards ------------------------------------------------------------

/// A library opened with the lock taken and the folder checked.
pub struct GuardedLibrary {
    pub opened: crate::OpenedLibrary,
    /// Held while this value lives; `None` when the library is read-only.
    pub lock: Option<LibraryLock>,
    /// Who else has the library open, when someone does.
    pub busy: Option<LockHolder>,
    pub warnings: Vec<FsWarning>,
}

impl GuardedLibrary {
    /// Whether changes are refused (somebody else has the library, or it was asked for).
    pub fn read_only(&self) -> bool {
        self.opened.library.is_read_only()
    }
}

/// Opens the library in `root`. If another live process holds it, or `read_only` is asked for,
/// the library is opened read-only (nothing is written, `busy` says who has it).
pub fn open_library_guarded(root: &Path, app: &str, read_only: bool) -> Result<GuardedLibrary> {
    let (lock, busy) = if read_only {
        (None, None)
    } else {
        match acquire(root, app)? {
            Lock::Acquired(l) => (Some(l), None),
            Lock::Busy(h) => (None, Some(h)),
        }
    };
    let mut opened = crate::open_library(root)?;
    if lock.is_none() {
        opened.library.set_read_only(true);
    }
    Ok(GuardedLibrary {
        opened,
        lock,
        busy,
        warnings: fs_warnings(root),
    })
}

// -- file system warnings ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsWarningKind {
    /// A network or FUSE-network file system.
    Network,
    /// A folder managed by a sync client.
    Sync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsWarning {
    pub kind: FsWarningKind,
    pub message: String,
}

fn is_network_fs(fstype: &str) -> bool {
    let t = fstype.to_ascii_lowercase();
    t.starts_with("nfs")
        || matches!(
            t.as_str(),
            "cifs"
                | "smb3"
                | "smbfs"
                | "afs"
                | "ceph"
                | "glusterfs"
                | "9p"
                | "ncpfs"
                | "davfs"
                | "lustre"
                | "fuse.sshfs"
                | "fuse.rclone"
                | "fuse.gvfsd-fuse"
                | "fuse.s3fs"
                | "fuse.davfs2"
                | "fuse.curlftpfs"
                | "fuse.smbnetfs"
        )
}

fn unescape_mount(s: &str) -> String {
    // /proc escapes space, tab, newline and backslash as \040-style octal
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 4], 8) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The file system type that holds `path`, from the text of `/proc/self/mountinfo`
/// (the mount with the longest mount point that contains the path wins).
pub fn fs_type_for(mountinfo: &str, path: &Path) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let mount_point = unescape_mount(left.split(' ').nth(4)?);
        let fstype = right.split(' ').next()?.to_string();
        if path.starts_with(&mount_point)
            && best
                .as_ref()
                .is_none_or(|(len, _)| mount_point.len() >= *len)
        {
            best = Some((mount_point.len(), fstype));
        }
    }
    best.map(|(_, t)| t)
}

/// The network file system type holding `path`, when it is one.
pub fn network_fs_type(mountinfo: &str, path: &Path) -> Option<String> {
    fs_type_for(mountinfo, path).filter(|t| is_network_fs(t))
}

/// The sync client that manages `root` or one of its parents: by marker files (`.dropbox`,
/// `.stfolder`, …) or by a typical folder name in the path.
pub fn sync_client(root: &Path, exists: &dyn Fn(&Path) -> bool) -> Option<&'static str> {
    const MARKERS: &[(&str, &str)] = &[
        (".dropbox", "Dropbox"),
        (".dropbox.cache", "Dropbox"),
        (".stfolder", "Syncthing"),
        (".stignore", "Syncthing"),
        (".sync", "Resilio Sync"),
        (".owncloudsync.log", "ownCloud/Nextcloud"),
        (".csync_journal.db", "ownCloud/Nextcloud"),
        (".nextcloudsync.log", "Nextcloud"),
    ];
    const NAMES: &[(&str, &str)] = &[
        ("dropbox", "Dropbox"),
        ("onedrive", "OneDrive"),
        ("google drive", "Google Drive"),
        ("googledrive", "Google Drive"),
        ("nextcloud", "Nextcloud"),
        ("owncloud", "ownCloud"),
        ("icloud drive", "iCloud Drive"),
        ("syncthing", "Syncthing"),
        ("resilio sync", "Resilio Sync"),
    ];
    for dir in root.ancestors().take(5) {
        for (marker, client) in MARKERS {
            if exists(&dir.join(marker)) {
                return Some(client);
            }
        }
    }
    for comp in root.components() {
        let c = comp.as_os_str().to_string_lossy().to_ascii_lowercase();
        for (name, client) in NAMES {
            if c == *name {
                return Some(client);
            }
        }
    }
    None
}

/// Everything worth warning about for the library folder `root`.
pub fn fs_warnings(root: &Path) -> Vec<FsWarning> {
    let path = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut out = Vec::new();
    if let Ok(info) = std::fs::read_to_string("/proc/self/mountinfo") {
        if let Some(t) = network_fs_type(&info, &path) {
            out.push(warning_network(&t));
        }
    }
    if let Some(c) = sync_client(&path, &|p| p.exists()) {
        out.push(warning_sync(c));
    }
    out
}

pub fn warning_network(fstype: &str) -> FsWarning {
    FsWarning {
        kind: FsWarningKind::Network,
        message: format!(
            "This library is on a network file system ({fstype}). SQLite can corrupt its database \
             there when two computers or a sleeping connection are involved. Keep the library on a \
             local disk and share it with “Share Library”, or make sure only one Bookx uses it."
        ),
    }
}

pub fn warning_sync(client: &str) -> FsWarning {
    FsWarning {
        kind: FsWarningKind::Sync,
        message: format!(
            "This library is inside a folder synced by {client}. Sync clients can copy the \
             database while it is being written and create conflicting copies. Exclude \
             bookx-library.db* from the sync, or keep the library outside it and use backups."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_first_process_gets_the_lock_and_the_second_is_told_who_has_it() {
        let d = tmp();
        let a = acquire_with(&d, "Bookx", 100, "box", &|_| true).unwrap();
        assert!(matches!(a, Lock::Acquired(_)));
        let Lock::Busy(h) = acquire_with(&d, "Bookx CLI", 200, "box", &|_| true).unwrap() else {
            panic!("expected busy")
        };
        assert_eq!(
            (h.pid, h.host.as_str(), h.app.as_str()),
            (100, "box", "Bookx")
        );
        let text = h.describe();
        assert!(
            text.contains("process 100") && text.contains("read-only") && text.contains(LOCK_FILE),
            "{text}"
        );
        drop(a);
        assert!(
            !d.join(LOCK_FILE).exists(),
            "the lock goes away with its owner"
        );
        assert!(matches!(
            acquire_with(&d, "Bookx", 200, "box", &|_| true).unwrap(),
            Lock::Acquired(_)
        ));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_lock_of_a_dead_process_on_this_machine_is_taken_over() {
        let d = tmp();
        let a = acquire_with(&d, "Bookx", 100, "box", &|_| true).unwrap();
        std::mem::forget(a); // a crash: the file stays
        assert!(d.join(LOCK_FILE).exists());
        let b = acquire_with(&d, "Bookx", 200, "box", &|pid| pid != 100).unwrap();
        assert!(matches!(b, Lock::Acquired(_)));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_lock_from_another_computer_is_never_taken_over() {
        let d = tmp();
        std::mem::forget(acquire_with(&d, "Bookx", 100, "laptop", &|_| true).unwrap());
        let r = acquire_with(&d, "Bookx", 200, "desktop", &|_| false).unwrap();
        assert!(matches!(r, Lock::Busy(h) if h.host == "laptop"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_damaged_lock_file_is_replaced() {
        let d = tmp();
        std::fs::write(d.join(LOCK_FILE), "garbage").unwrap();
        assert!(matches!(
            acquire_with(&d, "Bookx", 1, "box", &|_| true).unwrap(),
            Lock::Acquired(_)
        ));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn dropping_a_stale_guard_does_not_remove_somebody_elses_lock() {
        let d = tmp();
        let Lock::Acquired(a) = acquire_with(&d, "Bookx", 100, "box", &|_| true).unwrap() else {
            panic!()
        };
        std::fs::remove_file(d.join(LOCK_FILE)).unwrap();
        let b = acquire_with(&d, "Bookx", 200, "box", &|_| true).unwrap();
        drop(a);
        assert!(d.join(LOCK_FILE).exists());
        drop(b);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_busy_library_opens_read_only_with_the_holder_named() {
        let d = tmp();
        let root = d.join("lib");
        let first = open_library_guarded(&root, "Bookx", false).unwrap();
        assert!(!first.read_only() && first.lock.is_some() && first.busy.is_none());
        let second = open_library_guarded(&root, "Bookx CLI", false).unwrap();
        assert!(second.read_only());
        assert_eq!(
            second.busy.as_ref().map(|h| h.pid),
            Some(std::process::id())
        );
        // asked-for read-only does not touch the lock
        let third = open_library_guarded(&root, "Viewer", true).unwrap();
        assert!(third.read_only() && third.busy.is_none());
        drop(first);
        let again = open_library_guarded(&root, "Bookx", false).unwrap();
        assert!(!again.read_only());
        let _ = std::fs::remove_dir_all(d);
    }

    const MOUNTINFO: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
40 22 0:35 / /mnt/nas rw,relatime shared:60 - nfs4 server:/export rw,vers=4.2
41 22 0:36 / /mnt/my\\040share rw,relatime - cifs //srv/share rw
42 22 0:37 / /home/me/fuse rw,nosuid - fuse.sshfs me@host: rw
43 40 0:38 / /mnt/nas/local rw - ext4 /dev/sdb1 rw
";

    #[test]
    fn network_mounts_are_recognised_by_the_longest_mount_point() {
        let t = |p: &str| network_fs_type(MOUNTINFO, Path::new(p));
        assert_eq!(t("/mnt/nas/books").as_deref(), Some("nfs4"));
        assert_eq!(t("/mnt/my share/lib").as_deref(), Some("cifs"));
        assert_eq!(t("/home/me/fuse/lib").as_deref(), Some("fuse.sshfs"));
        assert_eq!(t("/home/me/lib"), None);
        assert_eq!(
            t("/mnt/nas/local/lib"),
            None,
            "a local disk mounted below a network mount"
        );
        assert_eq!(
            fs_type_for(MOUNTINFO, Path::new("/home/me")).as_deref(),
            Some("ext4")
        );
        assert_eq!(network_fs_type("", Path::new("/x")), None);
        assert_eq!(network_fs_type("garbage line", Path::new("/x")), None);
    }

    #[test]
    fn sync_folders_are_found_by_marker_or_name() {
        let none = |_: &Path| false;
        assert_eq!(
            sync_client(Path::new("/home/me/Documents/Bookx"), &none),
            None
        );
        assert_eq!(
            sync_client(Path::new("/home/me/Dropbox/Books"), &none),
            Some("Dropbox")
        );
        let marker = |p: &Path| p == Path::new("/data/sync/.stfolder");
        assert_eq!(
            sync_client(Path::new("/data/sync/books/lib"), &marker),
            Some("Syncthing")
        );
        assert_eq!(
            sync_client(Path::new("/home/me/Nextcloud"), &none),
            Some("Nextcloud")
        );
    }

    #[test]
    fn warnings_explain_the_risk_and_the_way_out() {
        let n = warning_network("nfs4");
        assert_eq!(n.kind, FsWarningKind::Network);
        assert!(n.message.contains("Share Library"));
        let s = warning_sync("Dropbox");
        assert!(s.message.contains("Dropbox") && s.message.contains("bookx-library.db"));
        // a plain temp dir is neither
        let d = tmp();
        let w = fs_warnings(&d);
        assert!(w.iter().all(|w| w.kind != FsWarningKind::Sync));
        let _ = std::fs::remove_dir_all(d);
    }
}
