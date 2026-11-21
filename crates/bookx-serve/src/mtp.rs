//! MTP devices (Android tablets and phones, many e-readers) through the desktop's gvfs mount.
//!
//! **Why not a Rust MTP crate?** `mtp-rs` (pure Rust, built on `nusb`) exists and is maintained,
//! but it talks to the USB device directly. On a GNOME desktop gvfs already holds the device open
//! (a second claim fails), and direct access needs udev rules or root. It is also young and
//! async-only. So this driver uses what the desktop already provides: gvfs mounts MTP devices as
//! ordinary directories under `/run/user/<uid>/gvfs/mtp:host=…/<storage>/`, and from there books
//! are plain files. `mtp-rs` stays the candidate if direct USB access is wanted later.
//!
//! Limits of the gvfs FUSE view: writes are sequential and slower than USB mass storage, there is
//! no `fsync` (so none is attempted), free space is often not reported, a device that locks its
//! screen shows no storage at all, and databases on the device (Kobo) cannot be used. Reading
//! state is only found where KOReader keeps sidecar folders.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bookx_core::contracts::{DeviceDriver, DeviceInfo};

use crate::devfs;
use crate::devices::{classify, sanitize_file_name, unique_name, DeviceKind};

/// Folders books go to, first existing one wins; `Books` is created when none exists.
const BOOK_FOLDERS: &[&str] = &["Books", "books", "eBooks", "Documents", "Download"];

/// `/run/user/<uid>/gvfs`, or `$XDG_RUNTIME_DIR/gvfs`.
pub fn default_gvfs_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d).join("gvfs"),
        None => PathBuf::from(format!("/run/user/{}/gvfs", user_id())),
    }
}

#[cfg(unix)]
fn user_id() -> u32 {
    nix::unistd::getuid().as_raw()
}

/// GVfs does not exist outside Unix; the path is never found.
#[cfg(not(unix))]
fn user_id() -> u32 {
    0
}

/// The driver. `gvfs_dir` holds the `mtp:host=…` mounts.
#[derive(Debug, Clone)]
pub struct MtpGvfsDriver {
    gvfs_dir: PathBuf,
}

impl Default for MtpGvfsDriver {
    fn default() -> Self {
        MtpGvfsDriver::new()
    }
}

impl MtpGvfsDriver {
    pub fn new() -> MtpGvfsDriver {
        MtpGvfsDriver {
            gvfs_dir: default_gvfs_dir(),
        }
    }

    pub fn with_gvfs_dir(dir: PathBuf) -> MtpGvfsDriver {
        MtpGvfsDriver { gvfs_dir: dir }
    }

    /// Kind of reader behind a storage (looked at the folder layout, `Generic` otherwise).
    pub fn kind_of(&self, device: &DeviceInfo) -> DeviceKind {
        classify(&device.mount).unwrap_or(DeviceKind::Generic)
    }

    /// Folder on the storage books are sent to.
    pub fn target_dir(&self, device: &DeviceInfo) -> PathBuf {
        match self.kind_of(device) {
            DeviceKind::Generic => {
                let sub = BOOK_FOLDERS
                    .iter()
                    .find(|f| device.mount.join(f).is_dir())
                    .copied()
                    .unwrap_or("Books");
                device.mount.join(sub)
            }
            k => device.mount.join(k.target_subdir(&device.mount)),
        }
    }

    /// Sends into folders below the target folder (see [`devfs::render_template`]).
    pub fn send_in(
        &self,
        device: &DeviceInfo,
        subdirs: &[String],
        file_name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        use std::io::Write;
        if file_name.trim().is_empty()
            || file_name.contains(['/', '\\', '\0'])
            || file_name == ".."
            || file_name == "."
        {
            bail!("refusing file name {file_name:?}");
        }
        if !device.mount.is_dir() {
            bail!(
                "the device is not mounted any more: {}",
                device.mount.display()
            );
        }
        let mut dir = self.target_dir(device);
        for d in subdirs {
            if d.is_empty() || d.contains(['/', '\\', '\0']) || d == ".." || d == "." {
                bail!("refusing folder name {d:?}");
            }
            dir.push(sanitize_file_name(d));
        }
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let name = sanitize_file_name(file_name);
        let mut candidate = unique_name(&dir, &name);
        for attempt in 0..50 {
            let path = dir.join(&candidate);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(bytes) {
                        drop(f);
                        let _ = std::fs::remove_file(&path);
                        return Err(e).with_context(|| format!("writing {}", path.display()));
                    }
                    // no sync_all: the gvfs FUSE layer sends the file when it is closed
                    drop(f);
                    return Ok(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    candidate = format!(
                        "{} ({}).{}",
                        name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name),
                        attempt + 2,
                        name.rsplit_once('.').map(|(_, e)| e).unwrap_or("epub")
                    );
                }
                Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
            }
        }
        bail!("could not find a free file name for {name}")
    }
}

/// `mtp:host=Pixel_7_ABC123` becomes "Pixel 7 ABC123".
fn pretty_host(dir_name: &str) -> String {
    let raw = dir_name
        .split("host=")
        .nth(1)
        .unwrap_or(dir_name)
        .trim_end_matches('/');
    let decoded = percent_encoding::percent_decode_str(raw)
        .decode_utf8_lossy()
        .replace('_', " ");
    decoded.trim().to_string()
}

impl DeviceDriver for MtpGvfsDriver {
    fn id(&self) -> &'static str {
        "mtp-gvfs"
    }

    fn detect(&self) -> Vec<DeviceInfo> {
        let Ok(rd) = std::fs::read_dir(&self.gvfs_dir) else {
            return Vec::new();
        };
        let mut hosts: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("mtp:"))
            .map(|e| e.path())
            .collect();
        hosts.sort();
        let mut out = Vec::new();
        for host in hosts {
            let host_name = pretty_host(&host.file_name().unwrap_or_default().to_string_lossy());
            // a locked phone lists no storage: nothing to offer, and no error either
            let Ok(rd) = std::fs::read_dir(&host) else {
                continue;
            };
            let mut storages: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            storages.sort();
            for mount in storages {
                let storage = mount
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push(DeviceInfo {
                    id: format!("mtp:{}", mount.display()),
                    name: format!("{host_name} – {storage} (MTP)"),
                    mount,
                });
            }
        }
        out
    }

    fn preferred_formats(&self) -> &'static [&'static str] {
        &["epub"]
    }

    fn send(&self, device: &DeviceInfo, file_name: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.send_in(device, &[], file_name, bytes)
    }

    fn list(&self, device: &DeviceInfo) -> Result<Vec<PathBuf>> {
        if !device.mount.is_dir() {
            bail!("the device is not mounted any more");
        }
        Ok(devfs::list_books(&device.mount)
            .into_iter()
            .map(|b| b.path)
            .collect())
    }
}

/// True if `path` looks like a gvfs MTP storage path (for messages).
pub fn is_gvfs_mtp(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with("mtp:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            let p = std::env::temp_dir().join(format!("bookx-mtp-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn storages_of_each_phone_are_listed() {
        let t = Tmp::new();
        t.dir("mtp:host=Pixel_7_ABC/Internal shared storage");
        t.dir("mtp:host=Pixel_7_ABC/SD card");
        t.dir("mtp:host=Locked_Phone");
        t.dir("smb-share:server=nas");
        std::fs::write(t.0.join("mtp:host=file"), b"x").unwrap();
        let d = MtpGvfsDriver::with_gvfs_dir(t.0.clone()).detect();
        let names: Vec<&str> = d.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Pixel 7 ABC – Internal shared storage (MTP)",
                "Pixel 7 ABC – SD card (MTP)"
            ]
        );
        assert!(d[0].id.starts_with("mtp:"));
        assert!(is_gvfs_mtp(&d[0].mount));
    }

    #[test]
    fn nothing_mounted_means_no_devices() {
        let d = MtpGvfsDriver::with_gvfs_dir(PathBuf::from("/nonexistent/gvfs"));
        assert!(d.detect().is_empty());
    }

    #[test]
    fn books_go_into_an_existing_book_folder_else_a_new_one() {
        let t = Tmp::new();
        let a = t.dir("mtp:host=A/Internal");
        let b = t.dir("mtp:host=B/Internal");
        t.dir("mtp:host=B/Internal/Documents");
        let drv = MtpGvfsDriver::with_gvfs_dir(t.0.clone());
        let devs = drv.detect();
        let pa = drv.send(&devs[0], "x.epub", b"1").unwrap();
        assert!(pa.starts_with(a.join("Books")), "{pa:?}");
        let pb = drv.send(&devs[1], "x.epub", b"1").unwrap();
        assert!(pb.starts_with(b.join("Documents")), "{pb:?}");
    }

    #[test]
    fn templates_make_folders_and_names_never_collide() {
        let t = Tmp::new();
        t.dir("mtp:host=A/Internal");
        let drv = MtpGvfsDriver::with_gvfs_dir(t.0.clone());
        let dev = drv.detect().remove(0);
        let sub = vec!["Ann".to_string(), "Series".to_string()];
        let p1 = drv.send_in(&dev, &sub, "One.epub", b"1").unwrap();
        let p2 = drv.send_in(&dev, &sub, "One.epub", b"2").unwrap();
        assert_ne!(p1, p2);
        assert!(p1.ends_with("Books/Ann/Series/One.epub"), "{p1:?}");
        assert_eq!(std::fs::read(&p1).unwrap(), b"1");
        assert_eq!(drv.list(&dev).unwrap().len(), 2);
    }

    #[test]
    fn hostile_names_and_unplugged_devices_are_errors() {
        let t = Tmp::new();
        t.dir("mtp:host=A/Internal");
        let drv = MtpGvfsDriver::with_gvfs_dir(t.0.clone());
        let dev = drv.detect().remove(0);
        assert!(drv.send(&dev, "../x.epub", b"1").is_err());
        assert!(drv.send(&dev, "", b"1").is_err());
        assert!(drv.send_in(&dev, &["..".into()], "x.epub", b"1").is_err());
        std::fs::remove_dir_all(&dev.mount).unwrap();
        assert!(drv.send(&dev, "x.epub", b"1").is_err());
        assert!(drv.list(&dev).is_err());
    }

    #[test]
    fn the_default_directory_follows_the_runtime_dir() {
        let d = default_gvfs_dir();
        assert!(d.ends_with("gvfs"));
    }
}
