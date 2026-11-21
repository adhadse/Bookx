//! USB mass-storage e-readers (Kobo, Kindle, PocketBook, Boox/generic).
//!
//! Detection scans a configurable list of mount roots (and `/proc/mounts` for removable file
//! systems), recognises the device by its directory layout, and copies books into the right folder
//! without ever overwriting an existing file. The Kobo database is only *detected*, not written.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use bookx_core::contracts::{DeviceDriver, DeviceInfo};

/// The kinds of reader this driver understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Kobo,
    Kindle,
    PocketBook,
    /// Boox and anything else that mounts as a disk with a `Books` folder.
    Generic,
}

impl DeviceKind {
    pub fn slug(&self) -> &'static str {
        match self {
            DeviceKind::Kobo => "kobo",
            DeviceKind::Kindle => "kindle",
            DeviceKind::PocketBook => "pocketbook",
            DeviceKind::Generic => "generic",
        }
    }

    pub fn from_slug(s: &str) -> Option<DeviceKind> {
        Some(match s {
            "kobo" => DeviceKind::Kobo,
            "kindle" => DeviceKind::Kindle,
            "pocketbook" => DeviceKind::PocketBook,
            "generic" => DeviceKind::Generic,
            _ => return None,
        })
    }

    pub fn label(&self) -> &'static str {
        match self {
            DeviceKind::Kobo => "Kobo",
            DeviceKind::Kindle => "Kindle",
            DeviceKind::PocketBook => "PocketBook",
            DeviceKind::Generic => "E-reader",
        }
    }

    /// Formats the device opens, best first.
    pub fn preferred_formats(&self) -> &'static [&'static str] {
        match self {
            DeviceKind::Kobo => &["kepub.epub", "epub"],
            DeviceKind::Kindle => &["azw3", "mobi", "epub"],
            DeviceKind::PocketBook => &["epub", "pdf", "fb2"],
            DeviceKind::Generic => &["epub"],
        }
    }

    /// Folder (relative to the mount) books are copied into. The Generic kind reuses an existing
    /// `Books` / `books` folder, keeping its case.
    pub fn target_subdir(&self, mount: &Path) -> PathBuf {
        match self {
            DeviceKind::Kobo => PathBuf::from("Bookx"),
            DeviceKind::Kindle => PathBuf::from("documents"),
            DeviceKind::PocketBook => PathBuf::from("Books"),
            DeviceKind::Generic => {
                if mount.join("books").is_dir() && !mount.join("Books").is_dir() {
                    PathBuf::from("books")
                } else {
                    PathBuf::from("Books")
                }
            }
        }
    }
}

/// First entry of the kind's preference list that is in `available` (case-insensitive).
pub fn choose_format<'a>(kind: DeviceKind, available: &[&'a str]) -> Option<&'a str> {
    kind.preferred_formats().iter().find_map(|want| {
        available
            .iter()
            .copied()
            .find(|have| have.eq_ignore_ascii_case(want))
    })
}

/// Recognises a device from its directory layout.
pub fn classify(root: &Path) -> Option<DeviceKind> {
    if root.join(".kobo").is_dir() {
        Some(DeviceKind::Kobo)
    } else if root.join("documents").is_dir() && root.join("system").is_dir() {
        Some(DeviceKind::Kindle)
    } else if root.join("system").join("config").exists() {
        Some(DeviceKind::PocketBook)
    } else if root.join("Books").is_dir() || root.join("books").is_dir() {
        Some(DeviceKind::Generic)
    } else {
        None
    }
}

/// Path of the Kobo library database if present (read-only knowledge: it is never written).
pub fn kobo_db_path(mount: &Path) -> Option<PathBuf> {
    let p = mount.join(".kobo").join("KoboReader.sqlite");
    p.is_file().then_some(p)
}

/// One line of `/proc/mounts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub source: String,
    pub mount_point: PathBuf,
    pub fstype: String,
    pub options: String,
}

fn unescape_mount_field(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 4 <= b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            let v = (b[i + 1] - b'0') as u32 * 64
                + (b[i + 2] - b'0') as u32 * 8
                + (b[i + 3] - b'0') as u32;
            if v <= 255 {
                out.push(v as u8);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses the text of `/proc/mounts`; malformed lines are skipped.
pub fn parse_proc_mounts(text: &str) -> Vec<MountEntry> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let source = f.next()?;
            let mp = f.next()?;
            let fstype = f.next()?;
            let options = f.next().unwrap_or("");
            Some(MountEntry {
                source: unescape_mount_field(source),
                mount_point: PathBuf::from(unescape_mount_field(mp)),
                fstype: fstype.to_string(),
                options: options.to_string(),
            })
        })
        .collect()
}

/// File systems e-readers use when they appear as a USB disk.
pub fn is_removable_fstype(fstype: &str) -> bool {
    matches!(
        fstype,
        "vfat" | "exfat" | "ntfs" | "ntfs3" | "fuseblk" | "msdos" | "udf" | "hfsplus" | "iso9660"
    )
}

/// Mount points of removable-looking file systems, excluding the boot partition.
pub fn removable_mounts(entries: &[MountEntry]) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|e| is_removable_fstype(&e.fstype))
        .filter(|e| !e.mount_point.starts_with("/boot") && !e.mount_point.starts_with("/efi"))
        .map(|e| e.mount_point.clone())
        .collect()
}

/// `/run/media/$USER`, `/media/$USER`, `/mnt` (the user part is skipped when unknown).
pub fn default_roots_for(user: Option<&str>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(u) = user.filter(|u| !u.is_empty() && !u.contains('/')) {
        roots.push(PathBuf::from("/run/media").join(u));
        roots.push(PathBuf::from("/media").join(u));
    } else {
        roots.push(PathBuf::from("/run/media"));
        roots.push(PathBuf::from("/media"));
    }
    roots.push(PathBuf::from("/mnt"));
    roots
}

/// macOS mounts every volume under `/Volumes/<name>`: the entries of that directory, without the
/// boot volume (a symlink to `/` named like `Macintosh HD`, which the caller filters out by passing
/// only real directories) and hidden entries. Pure: the caller lists the directory.
pub fn macos_volume_roots(volume_names: &[String]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = volume_names
        .iter()
        .filter(|n| !n.is_empty() && !n.starts_with('.') && !n.contains('/') && *n != "..")
        .filter(|n| {
            !n.starts_with("Macintosh HD")
                && !matches!(n.as_str(), "Recovery" | "Preboot" | "VM" | "Update")
        })
        .map(|n| PathBuf::from("/Volumes").join(n))
        .collect();
    v.sort();
    v
}

/// Windows exposes removable drives as `E:\`, `F:\`...: the drive letters D to Z that
/// `is_present` reports (C is the system drive, A and B are legacy floppies). Pure: the caller
/// supplies the existence check.
pub fn windows_drive_roots(is_present: impl Fn(char) -> bool) -> Vec<PathBuf> {
    ('D'..='Z')
        .filter(|c| is_present(*c))
        .map(|c| PathBuf::from(format!("{c}:\\")))
        .collect()
}

/// Where to look for mounted readers on this operating system: `(roots, mounts)`. A *root* is a
/// directory whose sub-directories are the mounts (Linux: `/run/media/$USER`, `/media/$USER`,
/// `/mnt`); a *mount* is a mount point itself (macOS: each entry of `/Volumes`; Windows: each present
/// drive letter D to Z).
pub fn platform_scan(user: Option<&str>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    if cfg!(target_os = "macos") {
        let names: Vec<String> = std::fs::read_dir("/Volumes")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir() && !e.path().is_symlink())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        (Vec::new(), macos_volume_roots(&names))
    } else if cfg!(windows) {
        (
            Vec::new(),
            windows_drive_roots(|c| Path::new(&format!("{c}:\\")).exists()),
        )
    } else {
        (default_roots_for(user), Vec::new())
    }
}

const MAX_NAME_BYTES: usize = 200;

/// Makes a name safe on FAT/exFAT/ext4 (no separators, reserved characters, control characters,
/// leading dots or spaces, trailing dots) and bounded in length, keeping the extension.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .to_string();
    // keep the extension when shortening
    let ext_len = ext_of(&cleaned).map(|e| e.len() + 1).unwrap_or(0).min(20);
    let mut out = cleaned;
    if out.len() > MAX_NAME_BYTES {
        let keep_ext = out[out.len() - ext_len..].to_string();
        let mut cut = MAX_NAME_BYTES - ext_len;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out = format!("{}{}", out.trim_end(), keep_ext);
    }
    if out.is_empty() {
        return "book".to_string();
    }
    out
}

fn ext_of(name: &str) -> Option<&str> {
    if name.to_ascii_lowercase().ends_with(".kepub.epub") {
        return Some(&name[name.len() - "kepub.epub".len()..]);
    }
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then_some(ext)
}

fn split_ext(name: &str) -> (&str, &str) {
    match ext_of(name) {
        Some(e) => (
            &name[..name.len() - e.len() - 1],
            &name[name.len() - e.len() - 1..],
        ),
        None => (name, ""),
    }
}

/// `name`, or `stem (1).ext`, `stem (2).ext`, ... the first one not present in `dir`.
pub fn unique_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    for n in 1..10_000 {
        let cand = format!("{stem} ({n}){ext}");
        if !dir.join(&cand).exists() {
            return cand;
        }
    }
    format!("{stem} ({}){ext}", uuid::Uuid::new_v4())
}

/// Rejects names that try to leave the target folder.
fn check_requested_name(file_name: &str) -> Result<()> {
    if file_name.trim().is_empty() {
        bail!("empty file name");
    }
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains('\0') {
        bail!("refusing file name with a path separator: {file_name:?}");
    }
    if matches!(
        Path::new(file_name).components().next(),
        Some(Component::ParentDir)
    ) || file_name == "."
        || file_name == ".."
    {
        bail!("refusing path traversal: {file_name:?}");
    }
    Ok(())
}

pub(crate) const BOOK_EXTENSIONS: &[&str] = &[
    "epub", "kepub", "mobi", "azw", "azw3", "pdf", "fb2", "txt", "cbz", "djvu",
];

fn collect_books(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let hidden = e.file_name().to_string_lossy().starts_with('.');
        if hidden {
            continue;
        }
        if p.is_dir() {
            if depth > 0 {
                collect_books(&p, depth - 1, out);
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| BOOK_EXTENSIONS.contains(&x.to_ascii_lowercase().as_str()))
        {
            out.push(p);
        }
    }
}

/// The USB mass-storage driver.
#[derive(Debug, Clone)]
pub struct MassStorageDriver {
    roots: Vec<PathBuf>,
    /// Mount points scanned directly (macOS volumes, Windows drives), not their sub-directories.
    mounts: Vec<PathBuf>,
    proc_mounts: Option<PathBuf>,
}

impl Default for MassStorageDriver {
    fn default() -> Self {
        MassStorageDriver::new()
    }
}

impl MassStorageDriver {
    /// Default roots for the current user and `/proc/mounts`.
    pub fn new() -> MassStorageDriver {
        let user = std::env::var("USER").ok();
        let (roots, mounts) = platform_scan(user.as_deref());
        MassStorageDriver {
            roots,
            mounts,
            proc_mounts: Some(PathBuf::from("/proc/mounts")),
        }
    }

    /// Custom roots and no `/proc/mounts` (tests, portable setups).
    pub fn with_roots(roots: Vec<PathBuf>) -> MassStorageDriver {
        MassStorageDriver {
            roots,
            mounts: Vec::new(),
            proc_mounts: None,
        }
    }

    /// File to read mount information from instead of `/proc/mounts`; `None` disables it.
    pub fn with_proc_mounts(mut self, path: Option<PathBuf>) -> MassStorageDriver {
        self.proc_mounts = path;
        self
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    fn candidates(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for root in &self.roots {
            let Ok(rd) = std::fs::read_dir(root) else {
                continue;
            };
            let mut dirs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            dirs.sort();
            out.extend(dirs);
        }
        out.extend(self.mounts.iter().filter(|m| m.is_dir()).cloned());
        if let Some(pm) = &self.proc_mounts {
            if let Ok(text) = std::fs::read_to_string(pm) {
                out.extend(removable_mounts(&parse_proc_mounts(&text)));
            }
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|p| {
            let key = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            seen.insert(key)
        });
        out
    }

    /// The kind of a device returned by [`DeviceDriver::detect`] (from its id, else by looking).
    pub fn kind_of(&self, device: &DeviceInfo) -> Option<DeviceKind> {
        device
            .id
            .split_once(':')
            .and_then(|(slug, _)| DeviceKind::from_slug(slug))
            .or_else(|| classify(&device.mount))
    }

    /// Formats this particular device opens, best first.
    pub fn formats_for(&self, device: &DeviceInfo) -> &'static [&'static str] {
        self.kind_of(device)
            .map(|k| k.preferred_formats())
            .unwrap_or(&["epub"])
    }

    /// Folder on the device books are sent to.
    pub fn target_dir(&self, device: &DeviceInfo) -> Result<PathBuf> {
        let kind = self.kind_of(device).context("not a recognised e-reader")?;
        Ok(device.mount.join(kind.target_subdir(&device.mount)))
    }
    /// Like [`DeviceDriver::send`], but into folders below the target folder (a save-path
    /// template, see [`crate::devfs::render_template`]). Every folder name is sanitised again here.
    pub fn send_in(
        &self,
        device: &DeviceInfo,
        subdirs: &[String],
        file_name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        check_requested_name(file_name)?;
        if !device.mount.is_dir() {
            bail!("device is not mounted: {}", device.mount.display());
        }
        let mut dir = self.target_dir(device)?;
        for d in subdirs {
            check_requested_name(d)?;
            dir.push(sanitize_file_name(d));
        }
        write_new_file(&device.mount, &dir, file_name, bytes)
    }
}

impl DeviceDriver for MassStorageDriver {
    fn id(&self) -> &'static str {
        "usb-mass-storage"
    }

    fn detect(&self) -> Vec<DeviceInfo> {
        self.candidates()
            .into_iter()
            .filter_map(|mount| {
                let kind = classify(&mount)?;
                let dir = mount
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| mount.display().to_string());
                Some(DeviceInfo {
                    id: format!("{}:{}", kind.slug(), mount.display()),
                    name: format!("{} ({dir})", kind.label()),
                    mount,
                })
            })
            .collect()
    }

    /// Driver-wide default; use [`MassStorageDriver::formats_for`] for a specific device.
    fn preferred_formats(&self) -> &'static [&'static str] {
        &["epub"]
    }

    fn send(&self, device: &DeviceInfo, file_name: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.send_in(device, &[], file_name, bytes)
    }

    fn list(&self, device: &DeviceInfo) -> Result<Vec<PathBuf>> {
        let dir = self.target_dir(device)?;
        let mut out = Vec::new();
        collect_books(&dir, 4, &mut out);
        out.sort();
        Ok(out)
    }
}

/// Creates `dir` (inside `mount`) and writes `bytes` as a new file named after `file_name`, never
/// overwriting: a taken name gets ` (1)`, ` (2)`…
pub(crate) fn write_new_file(
    mount: &Path,
    dir: &Path,
    file_name: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let real_dir = std::fs::canonicalize(dir)?;
    let real_mount = std::fs::canonicalize(mount)?;
    if !real_dir.starts_with(&real_mount) {
        bail!("target folder escapes the device");
    }
    let name = sanitize_file_name(file_name);
    let mut candidate = unique_name(&real_dir, &name);
    let mut tries = 0;
    loop {
        let path = real_dir.join(&candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(bytes).and_then(|_| f.sync_all()) {
                    drop(f);
                    let _ = std::fs::remove_file(&path);
                    return Err(e).with_context(|| format!("writing {}", path.display()));
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && tries < 100 => {
                tries += 1;
                let (stem, ext) = split_ext(&name);
                candidate = format!("{stem} ({}){ext}", tries + 1000);
            }
            Err(e) => {
                return Err(e).with_context(|| format!("creating {}", path.display()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            let p = std::env::temp_dir().join(format!("bookx-dev-{}", uuid::Uuid::new_v4()));
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

    fn kobo(t: &Tmp, name: &str) -> PathBuf {
        let m = t.dir(&format!("media/{name}"));
        std::fs::create_dir_all(m.join(".kobo")).unwrap();
        m
    }

    fn driver(t: &Tmp) -> MassStorageDriver {
        MassStorageDriver::with_roots(vec![t.0.join("media")])
    }

    #[test]
    fn slugs_roundtrip() {
        for k in [
            DeviceKind::Kobo,
            DeviceKind::Kindle,
            DeviceKind::PocketBook,
            DeviceKind::Generic,
        ] {
            assert_eq!(DeviceKind::from_slug(k.slug()), Some(k));
            assert!(!k.label().is_empty());
        }
        assert_eq!(DeviceKind::from_slug("nope"), None);
    }

    #[test]
    fn preferred_formats_per_kind() {
        assert_eq!(
            DeviceKind::Kobo.preferred_formats(),
            &["kepub.epub", "epub"]
        );
        assert_eq!(
            DeviceKind::Kindle.preferred_formats(),
            &["azw3", "mobi", "epub"]
        );
        assert_eq!(DeviceKind::Generic.preferred_formats(), &["epub"]);
        assert_eq!(DeviceKind::PocketBook.preferred_formats()[0], "epub");
    }

    #[test]
    fn choose_format_follows_preference_order() {
        assert_eq!(
            choose_format(DeviceKind::Kobo, &["epub", "kepub.epub"]),
            Some("kepub.epub")
        );
        assert_eq!(choose_format(DeviceKind::Kobo, &["epub"]), Some("epub"));
        assert_eq!(
            choose_format(DeviceKind::Kindle, &["epub", "MOBI"]),
            Some("MOBI")
        );
        assert_eq!(choose_format(DeviceKind::Kindle, &["pdf"]), None);
        assert_eq!(choose_format(DeviceKind::Generic, &[]), None);
    }

    #[test]
    fn classify_each_kind_and_unknown() {
        let t = Tmp::new();
        let kobo = t.dir("a");
        std::fs::create_dir_all(kobo.join(".kobo")).unwrap();
        assert_eq!(classify(&kobo), Some(DeviceKind::Kobo));

        let kindle = t.dir("b");
        std::fs::create_dir_all(kindle.join("documents")).unwrap();
        assert_eq!(classify(&kindle), None); // documents alone is not enough
        std::fs::create_dir_all(kindle.join("system")).unwrap();
        assert_eq!(classify(&kindle), Some(DeviceKind::Kindle));

        let pb = t.dir("c");
        std::fs::create_dir_all(pb.join("system/config")).unwrap();
        assert_eq!(classify(&pb), Some(DeviceKind::PocketBook));

        let generic = t.dir("d");
        std::fs::create_dir_all(generic.join("books")).unwrap();
        assert_eq!(classify(&generic), Some(DeviceKind::Generic));
        let generic2 = t.dir("e");
        std::fs::create_dir_all(generic2.join("Books")).unwrap();
        assert_eq!(classify(&generic2), Some(DeviceKind::Generic));

        assert_eq!(classify(&t.dir("empty")), None);
        assert_eq!(classify(&t.0.join("missing")), None);
    }

    #[test]
    fn kobo_wins_over_other_markers() {
        let t = Tmp::new();
        let m = t.dir("m");
        std::fs::create_dir_all(m.join(".kobo")).unwrap();
        std::fs::create_dir_all(m.join("Books")).unwrap();
        assert_eq!(classify(&m), Some(DeviceKind::Kobo));
    }

    #[test]
    fn kobo_db_detected_by_presence_only() {
        let t = Tmp::new();
        let m = kobo(&t, "KOBOeReader");
        assert_eq!(kobo_db_path(&m), None);
        std::fs::write(m.join(".kobo/KoboReader.sqlite"), b"SQLite format 3\0").unwrap();
        assert_eq!(kobo_db_path(&m), Some(m.join(".kobo/KoboReader.sqlite")));
        // detection never modifies it
        assert_eq!(
            std::fs::read(m.join(".kobo/KoboReader.sqlite")).unwrap(),
            b"SQLite format 3\0"
        );
    }

    #[test]
    fn target_dirs() {
        let t = Tmp::new();
        let m = t.dir("g");
        assert_eq!(DeviceKind::Kobo.target_subdir(&m), PathBuf::from("Bookx"));
        assert_eq!(
            DeviceKind::Kindle.target_subdir(&m),
            PathBuf::from("documents")
        );
        assert_eq!(
            DeviceKind::PocketBook.target_subdir(&m),
            PathBuf::from("Books")
        );
        assert_eq!(
            DeviceKind::Generic.target_subdir(&m),
            PathBuf::from("Books")
        );
        std::fs::create_dir_all(m.join("books")).unwrap();
        assert_eq!(
            DeviceKind::Generic.target_subdir(&m),
            PathBuf::from("books")
        );
    }

    #[test]
    fn parses_proc_mounts() {
        let text = "\
sysfs /sys sysfs rw,nosuid 0 0
/dev/sda1 /run/media/me/KOBOeReader vfat rw,nosuid,nodev 0 0
/dev/sdb1 /run/media/me/My\\040Kindle exfat rw 0 0
/dev/nvme0n1p1 /boot/efi vfat rw 0 0
/dev/nvme0n1p2 / ext4 rw 0 0
garbage
";
        let e = parse_proc_mounts(text);
        assert_eq!(e.len(), 5);
        assert_eq!(e[1].source, "/dev/sda1");
        assert_eq!(e[1].fstype, "vfat");
        assert_eq!(e[1].mount_point, PathBuf::from("/run/media/me/KOBOeReader"));
        assert_eq!(e[2].mount_point, PathBuf::from("/run/media/me/My Kindle"));
        assert_eq!(e[1].options, "rw,nosuid,nodev");
        let removable = removable_mounts(&e);
        assert_eq!(
            removable,
            vec![
                PathBuf::from("/run/media/me/KOBOeReader"),
                PathBuf::from("/run/media/me/My Kindle")
            ]
        );
    }

    #[test]
    fn proc_mounts_edge_cases() {
        assert!(parse_proc_mounts("").is_empty());
        assert!(parse_proc_mounts("\n\n  \n").is_empty());
        assert_eq!(unescape_mount_field("a\\040b\\011c\\134d"), "a b\tc\\d");
        // incomplete / non-octal escapes stay as they are
        assert_eq!(unescape_mount_field("a\\04"), "a\\04");
        assert_eq!(unescape_mount_field("a\\089"), "a\\089");
        assert_eq!(unescape_mount_field("trail\\"), "trail\\");
        // out of byte range
        assert_eq!(unescape_mount_field("\\777"), "\\777");
        // unicode passes through
        assert_eq!(unescape_mount_field("日本"), "日本");
        let one = parse_proc_mounts("/dev/x /mnt/x ntfs3");
        assert_eq!(one[0].options, "");
    }

    #[test]
    fn removable_fs_types() {
        for t in ["vfat", "exfat", "ntfs", "ntfs3", "fuseblk", "udf"] {
            assert!(is_removable_fstype(t), "{t}");
        }
        for t in ["ext4", "btrfs", "tmpfs", "proc", "xfs"] {
            assert!(!is_removable_fstype(t), "{t}");
        }
    }

    #[test]
    fn macos_volumes_skip_boot_and_hidden_entries() {
        let names: Vec<String> = [
            "KOBOeReader",
            "Macintosh HD",
            "Macintosh HD - Data",
            ".timemachine",
            "Kindle",
            "Recovery",
            "a/b",
            "",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            macos_volume_roots(&names),
            vec![
                PathBuf::from("/Volumes/KOBOeReader"),
                PathBuf::from("/Volumes/Kindle")
            ]
        );
        assert!(macos_volume_roots(&[]).is_empty());
    }

    #[test]
    fn windows_drives_are_d_to_z_and_present() {
        let roots = windows_drive_roots(|c| matches!(c, 'C' | 'E' | 'G' | 'a'));
        assert_eq!(roots, vec![PathBuf::from("E:\\"), PathBuf::from("G:\\")]);
        assert!(windows_drive_roots(|_| false).is_empty());
        assert_eq!(windows_drive_roots(|_| true).len(), 23);
    }

    #[test]
    fn platform_scan_on_linux_is_the_media_directories() {
        if cfg!(target_os = "linux") {
            assert_eq!(
                platform_scan(Some("anna")),
                (default_roots_for(Some("anna")), Vec::new())
            );
        }
    }

    #[test]
    fn default_roots() {
        assert_eq!(
            default_roots_for(Some("anna")),
            vec![
                PathBuf::from("/run/media/anna"),
                PathBuf::from("/media/anna"),
                PathBuf::from("/mnt")
            ]
        );
        assert_eq!(
            default_roots_for(None),
            vec![
                PathBuf::from("/run/media"),
                PathBuf::from("/media"),
                PathBuf::from("/mnt")
            ]
        );
        // a hostile user name cannot add path segments
        assert_eq!(default_roots_for(Some("../x")), default_roots_for(None));
        assert_eq!(default_roots_for(Some("")), default_roots_for(None));
        assert!(!MassStorageDriver::new().roots().is_empty());
    }

    #[test]
    fn sanitizer() {
        assert_eq!(
            sanitize_file_name("Dune: Part 1?.epub"),
            "Dune_ Part 1_.epub"
        );
        assert_eq!(sanitize_file_name("a/b\\c.epub"), "a_b_c.epub");
        assert_eq!(sanitize_file_name("  ..hidden.epub  "), "hidden.epub");
        assert_eq!(sanitize_file_name("tab\there\n.epub"), "tab_here_.epub");
        assert_eq!(sanitize_file_name(""), "book");
        assert_eq!(sanitize_file_name("..."), "book");
        assert_eq!(sanitize_file_name("日本語の本.epub"), "日本語の本.epub");
        assert_eq!(sanitize_file_name("x.kepub.epub"), "x.kepub.epub");
        assert_eq!(sanitize_file_name("a|b*c\"d<e>f"), "a_b_c_d_e_f");
    }

    #[test]
    fn sanitizer_limits_length_and_keeps_extension() {
        let long = format!("{}.epub", "é".repeat(300));
        let s = sanitize_file_name(&long);
        assert!(s.len() <= MAX_NAME_BYTES, "{}", s.len());
        assert!(s.ends_with(".epub"));
        assert!(s.is_char_boundary(s.len()));
        let long_kepub = format!("{}.kepub.epub", "x".repeat(400));
        let s = sanitize_file_name(&long_kepub);
        assert!(s.ends_with(".kepub.epub"));
        assert!(s.len() <= MAX_NAME_BYTES);
    }

    #[test]
    fn unique_names_get_suffixes() {
        let t = Tmp::new();
        let d = t.dir("u");
        assert_eq!(unique_name(&d, "a.epub"), "a.epub");
        std::fs::write(d.join("a.epub"), b"1").unwrap();
        assert_eq!(unique_name(&d, "a.epub"), "a (1).epub");
        std::fs::write(d.join("a (1).epub"), b"1").unwrap();
        assert_eq!(unique_name(&d, "a.epub"), "a (2).epub");
        std::fs::write(d.join("k.kepub.epub"), b"1").unwrap();
        assert_eq!(unique_name(&d, "k.kepub.epub"), "k (1).kepub.epub");
        std::fs::write(d.join("noext"), b"1").unwrap();
        assert_eq!(unique_name(&d, "noext"), "noext (1)");
    }

    #[test]
    fn detect_finds_devices_in_all_roots_and_ignores_others() {
        let t = Tmp::new();
        kobo(&t, "KOBOeReader");
        let kindle = t.dir("media/Kindle");
        std::fs::create_dir_all(kindle.join("documents")).unwrap();
        std::fs::create_dir_all(kindle.join("system")).unwrap();
        t.dir("media/USBSTICK");
        std::fs::write(t.0.join("media/loosefile"), b"x").unwrap();
        let d = driver(&t);
        let found = d.detect();
        assert_eq!(found.len(), 2);
        let kobo = found.iter().find(|f| f.id.starts_with("kobo:")).unwrap();
        assert_eq!(kobo.mount, t.0.join("media/KOBOeReader"));
        assert_eq!(kobo.name, "Kobo (KOBOeReader)");
        assert!(found.iter().any(|f| f.id.starts_with("kindle:")));
        assert_eq!(d.id(), "usb-mass-storage");
    }

    #[test]
    fn detect_scans_direct_mounts_like_macos_volumes_and_windows_drives() {
        let t = Tmp::new();
        let kobo_mount = t.dir("Volumes/KOBOeReader");
        std::fs::create_dir_all(kobo_mount.join(".kobo")).unwrap();
        let other = t.dir("Volumes/Backup");
        let mut d = MassStorageDriver::with_roots(vec![]);
        d.mounts = vec![kobo_mount.clone(), other, t.0.join("Volumes/Missing")];
        let found = d.detect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].mount, kobo_mount);
        assert!(found[0].id.starts_with("kobo:"));
    }

    #[test]
    fn detect_with_missing_root_is_empty() {
        let d = MassStorageDriver::with_roots(vec![PathBuf::from("/definitely/not/here")]);
        assert!(d.detect().is_empty());
        assert!(MassStorageDriver::with_roots(vec![]).detect().is_empty());
    }

    #[test]
    fn detect_uses_proc_mounts_and_dedups() {
        let t = Tmp::new();
        let m = kobo(&t, "KOBO");
        let pocket = t.dir("elsewhere/PB");
        std::fs::create_dir_all(pocket.join("system/config")).unwrap();
        let ext4 = t.dir("elsewhere/internal");
        std::fs::create_dir_all(ext4.join("Books")).unwrap();
        let mounts = t.0.join("mounts");
        std::fs::write(
            &mounts,
            format!(
                "/dev/sda1 {} vfat rw 0 0\n/dev/sdb1 {} exfat rw 0 0\n/dev/sdc1 {} ext4 rw 0 0\n",
                m.display(),
                pocket.display(),
                ext4.display()
            ),
        )
        .unwrap();
        let d = driver(&t).with_proc_mounts(Some(mounts));
        let found = d.detect();
        // KOBO appears under the root and in /proc/mounts: reported once. ext4 is not removable.
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().any(|f| f.id.starts_with("pocketbook:")));
        assert_eq!(
            found.iter().filter(|f| f.id.starts_with("kobo:")).count(),
            1
        );
    }

    #[test]
    fn detect_ignores_unreadable_proc_mounts() {
        let t = Tmp::new();
        kobo(&t, "K");
        let d = driver(&t).with_proc_mounts(Some(t.0.join("no-such-file")));
        assert_eq!(d.detect().len(), 1);
    }

    #[test]
    fn send_to_kobo_goes_to_bookx_folder() {
        let t = Tmp::new();
        kobo(&t, "KOBO");
        let d = driver(&t);
        let dev = d.detect().remove(0);
        let p = d.send(&dev, "Dune.kepub.epub", b"DATA").unwrap();
        assert!(
            p.ends_with("KOBO/Bookx/Dune.kepub.epub")
                || p.to_string_lossy().ends_with("/Bookx/Dune.kepub.epub")
        );
        assert_eq!(std::fs::read(&p).unwrap(), b"DATA");
        assert_eq!(d.formats_for(&dev), &["kepub.epub", "epub"]);
    }

    #[test]
    fn send_to_kindle_goes_to_documents() {
        let t = Tmp::new();
        let k = t.dir("media/Kindle");
        std::fs::create_dir_all(k.join("documents")).unwrap();
        std::fs::create_dir_all(k.join("system")).unwrap();
        let d = driver(&t);
        let dev = d.detect().remove(0);
        let p = d.send(&dev, "b.azw3", b"x").unwrap();
        assert!(p.to_string_lossy().ends_with("/documents/b.azw3"));
        assert_eq!(d.formats_for(&dev), &["azw3", "mobi", "epub"]);
    }

    #[test]
    fn send_to_generic_reuses_existing_lowercase_books() {
        let t = Tmp::new();
        let g = t.dir("media/BOOX");
        std::fs::create_dir_all(g.join("books")).unwrap();
        let d = driver(&t);
        let dev = d.detect().remove(0);
        let p = d.send(&dev, "x.epub", b"1").unwrap();
        assert!(p.to_string_lossy().ends_with("/books/x.epub"));
        assert_eq!(d.formats_for(&dev), &["epub"]);
        assert_eq!(d.preferred_formats(), &["epub"]);
    }

    #[test]
    fn send_never_overwrites() {
        let t = Tmp::new();
        kobo(&t, "KOBO");
        let d = driver(&t);
        let dev = d.detect().remove(0);
        let a = d.send(&dev, "same.epub", b"first").unwrap();
        let b = d.send(&dev, "same.epub", b"second").unwrap();
        let c = d.send(&dev, "same.epub", b"third").unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(std::fs::read(&a).unwrap(), b"first");
        assert_eq!(std::fs::read(&b).unwrap(), b"second");
        assert_eq!(std::fs::read(&c).unwrap(), b"third");
        assert!(b.to_string_lossy().ends_with("same (1).epub"));
    }

    #[test]
    fn send_refuses_path_traversal() {
        let t = Tmp::new();
        kobo(&t, "KOBO");
        let d = driver(&t);
        let dev = d.detect().remove(0);
        for bad in [
            "../evil.epub",
            "..",
            ".",
            "a/b.epub",
            "a\\b.epub",
            "",
            "  ",
            "x\0.epub",
            "/etc/passwd",
        ] {
            let err = d.send(&dev, bad, b"x").unwrap_err();
            assert!(!format!("{err}").is_empty(), "{bad:?}");
        }
        let err = d.send(&dev, "../evil.epub", b"x").unwrap_err();
        assert!(format!("{err}").contains("path separator"));
        assert!(!t.0.join("media/evil.epub").exists());
        assert!(!t.0.join("media/KOBO/evil.epub").exists());
        assert!(d.list(&dev).unwrap().is_empty());
    }

    #[test]
    fn send_sanitises_unsafe_characters() {
        let t = Tmp::new();
        kobo(&t, "KOBO");
        let d = driver(&t);
        let dev = d.detect().remove(0);
        let p = d.send(&dev, "What? A *Book*: \"Yes\".epub", b"x").unwrap();
        assert!(p.file_name().unwrap().to_string_lossy() == "What_ A _Book__ _Yes_.epub");
    }

    #[test]
    fn send_to_unmounted_device_fails() {
        let t = Tmp::new();
        let d = driver(&t);
        let dev = DeviceInfo {
            id: "kobo:/gone".into(),
            name: "gone".into(),
            mount: t.0.join("gone"),
        };
        let err = d.send(&dev, "a.epub", b"x").unwrap_err();
        assert!(format!("{err}").contains("not mounted"));
    }

    #[test]
    fn send_to_unrecognised_mount_fails() {
        let t = Tmp::new();
        let m = t.dir("media/plain");
        let d = driver(&t);
        let dev = DeviceInfo {
            id: "weird".into(),
            name: "x".into(),
            mount: m,
        };
        let err = d.send(&dev, "a.epub", b"x").unwrap_err();
        assert!(format!("{err}").contains("not a recognised"));
        assert_eq!(d.formats_for(&dev), &["epub"]);
    }

    #[cfg(unix)]
    #[test]
    fn send_refuses_symlink_escape() {
        let t = Tmp::new();
        let m = kobo(&t, "KOBO");
        let outside = t.dir("outside");
        std::os::unix::fs::symlink(&outside, m.join("Bookx")).unwrap();
        let d = driver(&t);
        let dev = d.detect().remove(0);
        let err = d.send(&dev, "a.epub", b"x").unwrap_err();
        assert!(format!("{err}").contains("escapes"));
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    }

    #[test]
    fn list_returns_books_recursively_and_skips_hidden() {
        let t = Tmp::new();
        kobo(&t, "KOBO");
        let d = driver(&t);
        let dev = d.detect().remove(0);
        assert!(d.list(&dev).unwrap().is_empty()); // folder does not exist yet
        d.send(&dev, "b.epub", b"1").unwrap();
        d.send(&dev, "a.kepub.epub", b"1").unwrap();
        let dir = d.target_dir(&dev).unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/c.PDF"), b"1").unwrap();
        std::fs::write(dir.join("notes.docx"), b"1").unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join(".hidden/h.epub"), b"1").unwrap();
        let names: Vec<String> = d
            .list(&dev)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.kepub.epub", "b.epub", "c.PDF"]);
    }

    #[test]
    fn kind_of_falls_back_to_layout() {
        let t = Tmp::new();
        let m = kobo(&t, "K");
        let d = driver(&t);
        let dev = DeviceInfo {
            id: "no-colon".into(),
            name: "n".into(),
            mount: m,
        };
        assert_eq!(d.kind_of(&dev), Some(DeviceKind::Kobo));
        let dev2 = DeviceInfo {
            id: "bogus:/x".into(),
            name: "n".into(),
            mount: t.0.clone(),
        };
        assert_eq!(d.kind_of(&dev2), None);
    }
}
