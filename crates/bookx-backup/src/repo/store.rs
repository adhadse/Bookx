//! Where a repository lives: a small file-store trait (folder, WebDAV, in memory) with atomic
//! `put` (an object is never visible half written), streaming transfer in both directions and
//! paths that are checked before they touch a disk or a URL.

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use bookx_serve::webdav::WebDav;

use crate::push::Cancelled;

/// The folder inside a place where the repository lives, next to the old `.bookxbackup` files.
pub const REPO_DIR: &str = "Bookx Backup";

/// One entry of a repository folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// A place a repository can be kept in. Paths are `/`-separated and relative to the repository
/// folder (`blobs/ab/<id>`); see [`valid_path`].
pub trait RepoStore: Send + Sync {
    /// "/mnt/nas/Bookx Backup" or "https://cloud.example.com/…".
    fn label(&self) -> String;
    /// Reaches the place and can write there, with the reason in plain words when not.
    fn check(&self) -> Result<()>;
    /// Free bytes, when the place can tell.
    fn free_space(&self) -> Option<u64> {
        None
    }
    /// The entries directly in `dir` ("" is the repository folder). A missing folder is empty.
    fn list(&self, dir: &str) -> Result<Vec<Entry>>;
    /// Writes exactly `len` bytes from `data` under `path`, replacing what is there. The object
    /// appears whole or not at all; `cancel` is looked at while copying (error: [`Cancelled`]).
    fn put(&self, path: &str, data: &mut dyn Read, len: u64, cancel: &AtomicBool) -> Result<()>;
    /// Streams the object into `out`; more than `max` bytes is an error.
    fn get_to(&self, path: &str, out: &mut dyn Write, max: u64) -> Result<u64>;
    /// Removes an object (missing is fine).
    fn delete(&self, path: &str) -> Result<()>;

    /// The size of the object at `path`, `None` when it is not there.
    fn stat(&self, path: &str) -> Result<Option<u64>> {
        let (dir, name) = split(path);
        Ok(self
            .list(dir)?
            .into_iter()
            .find(|e| !e.is_dir && e.name == name)
            .map(|e| e.size))
    }

    /// Whole small object into memory (at most `max` bytes).
    fn get_bytes(&self, path: &str, max: u64) -> Result<Vec<u8>> {
        let mut v = Vec::new();
        self.get_to(path, &mut v, max)?;
        Ok(v)
    }

    fn put_bytes(&self, path: &str, data: &[u8], cancel: &AtomicBool) -> Result<()> {
        self.put(
            path,
            &mut std::io::Cursor::new(data),
            data.len() as u64,
            cancel,
        )
    }
}

impl<T: RepoStore + ?Sized> RepoStore for std::sync::Arc<T> {
    fn label(&self) -> String {
        (**self).label()
    }
    fn check(&self) -> Result<()> {
        (**self).check()
    }
    fn free_space(&self) -> Option<u64> {
        (**self).free_space()
    }
    fn list(&self, dir: &str) -> Result<Vec<Entry>> {
        (**self).list(dir)
    }
    fn put(&self, path: &str, data: &mut dyn Read, len: u64, cancel: &AtomicBool) -> Result<()> {
        (**self).put(path, data, len, cancel)
    }
    fn get_to(&self, path: &str, out: &mut dyn Write, max: u64) -> Result<u64> {
        (**self).get_to(path, out, max)
    }
    fn delete(&self, path: &str) -> Result<()> {
        (**self).delete(path)
    }
}

/// `("blobs/ab", "id")` for `blobs/ab/id`, `("", "x")` for `x`.
pub fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// The paths a repository uses: up to three segments of letters, digits, `.`, `-`, `_`, never
/// `..`. Nothing else is ever turned into a file name or a URL.
pub fn valid_path(path: &str) -> bool {
    let segs: Vec<&str> = path.split('/').collect();
    !segs.is_empty()
        && segs.len() <= 3
        && segs.iter().all(|s| {
            !s.is_empty()
                && s.len() <= 120
                && *s != "."
                && *s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
}

fn check_path(path: &str) -> Result<()> {
    if valid_path(path) {
        Ok(())
    } else {
        bail!("{path:?} is not a repository path")
    }
}

fn check_dir(dir: &str) -> Result<()> {
    if dir.is_empty() {
        Ok(())
    } else {
        check_path(dir)
    }
}

/// Copies `src` to `dst`, checking `cancel` between blocks, until `len` bytes are done.
fn copy_exact(
    src: &mut dyn Read,
    dst: &mut dyn Write,
    len: u64,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut buf = vec![0u8; 256 * 1024];
    let mut left = len;
    while left > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow::Error::new(Cancelled));
        }
        let want = buf.len().min(left as usize);
        let n = match src.read(&mut buf[..want]) {
            Ok(0) => bail!("the data ended before its announced length"),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(anyhow::Error::new(Cancelled));
                }
                return Err(anyhow::Error::new(e));
            }
        };
        dst.write_all(&buf[..n])?;
        left -= n as u64;
    }
    // the source must be at its end: a longer source is a file that changed
    let mut probe = [0u8; 1];
    match src.read(&mut probe) {
        Ok(0) => Ok(()),
        Ok(_) => bail!("the data is longer than its announced length"),
        Err(e) => Err(anyhow::Error::new(e)),
    }
}

/// Copies at most `max` bytes; one more is an error.
fn copy_limited(src: &mut dyn Read, dst: &mut dyn Write, max: u64) -> Result<u64> {
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(anyhow::Error::new(e)),
        };
        total += n as u64;
        if total > max {
            bail!("the stored object is bigger than it should be");
        }
        dst.write_all(&buf[..n])?;
    }
}

// -- a folder -----------------------------------------------------------------------------------------

/// A repository in a local folder: a disk, a mounted share, a cloud-drive mount.
pub struct FolderStore {
    root: PathBuf,
}

impl FolderStore {
    /// `root` is the repository folder itself (`…/Bookx Backup`).
    pub fn new(root: impl Into<PathBuf>) -> FolderStore {
        FolderStore { root: root.into() }
    }

    fn file(&self, path: &str) -> Result<PathBuf> {
        check_path(path)?;
        Ok(self.root.join(path))
    }
}

fn refuse_symlink(p: &std::path::Path) -> Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => {
            bail!("{} is a link; refusing to follow it", p.display())
        }
        _ => Ok(()),
    }
}

impl RepoStore for FolderStore {
    fn label(&self) -> String {
        self.root.display().to_string()
    }

    fn check(&self) -> Result<()> {
        if !self.root.exists() {
            let parent = self.root.parent().filter(|p| !p.as_os_str().is_empty());
            if parent.is_some_and(|p| !p.is_dir()) {
                bail!(
                    "{} is not there. Is the drive plugged in or the share mounted?",
                    self.root.display()
                );
            }
        } else if !self.root.is_dir() {
            bail!("{} is not a folder", self.root.display());
        }
        refuse_symlink(&self.root)?;
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("cannot create {}", self.root.display()))?;
        let probe = self.root.join(".bookx-write-test");
        std::fs::write(&probe, b"x")
            .with_context(|| format!("cannot write in {}", self.root.display()))?;
        let _ = std::fs::remove_file(&probe);
        Ok(())
    }

    fn free_space(&self) -> Option<u64> {
        let mut p = self.root.as_path();
        while !p.exists() {
            p = p.parent()?;
        }
        bookx_serve::devfs::free_space(p).map(|s| s.free)
    }

    fn list(&self, dir: &str) -> Result<Vec<Entry>> {
        check_dir(dir)?;
        let d = self.root.join(dir);
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(anyhow!("cannot read {}: {e}", d.display())),
        };
        Ok(rd
            .flatten()
            .filter_map(|e| {
                let ft = e.file_type().ok()?;
                if ft.is_symlink() {
                    return None;
                }
                Some(Entry {
                    name: e.file_name().to_string_lossy().to_string(),
                    size: e.metadata().map(|m| m.len()).unwrap_or(0),
                    is_dir: ft.is_dir(),
                })
            })
            .collect())
    }

    fn put(&self, path: &str, data: &mut dyn Read, len: u64, cancel: &AtomicBool) -> Result<()> {
        let dest = self.file(path)?;
        let parent = dest.parent().expect("repository paths have a parent");
        refuse_symlink(parent)?;
        std::fs::create_dir_all(parent)?;
        refuse_symlink(parent)?;
        let name = dest.file_name().unwrap().to_string_lossy().to_string();
        let part = parent.join(format!(".{name}.part-{}", super::seal::random_hex(4)?));
        let result = (|| -> Result<()> {
            let mut f = std::fs::File::create_new(&part)
                .with_context(|| format!("writing {}", part.display()))?;
            copy_exact(data, &mut f, len, cancel)?;
            f.sync_all().ok();
            drop(f);
            if std::fs::rename(&part, &dest).is_err() {
                // some mounts (FUSE) refuse to rename over a file: replace it directly
                let _ = std::fs::remove_file(&dest);
                std::fs::rename(&part, &dest)
                    .with_context(|| format!("moving {} into place", dest.display()))?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        result
    }

    fn get_to(&self, path: &str, out: &mut dyn Write, max: u64) -> Result<u64> {
        let p = self.file(path)?;
        let m = std::fs::symlink_metadata(&p).with_context(|| format!("reading {path}"))?;
        if !m.is_file() {
            bail!("{path} is not a plain file");
        }
        let mut f = std::fs::File::open(&p).with_context(|| format!("reading {path}"))?;
        copy_limited(&mut f, out, max)
    }

    fn delete(&self, path: &str) -> Result<()> {
        let p = self.file(path)?;
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() => bail!("{path} is a folder"),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(anyhow!("cannot delete {path}: {e}")),
        }
        std::fs::remove_file(&p).map_err(|e| anyhow!("cannot delete {path}: {e}"))
    }
}

// -- WebDAV -------------------------------------------------------------------------------------------

/// A repository in a WebDAV folder. Objects are uploaded to a temporary name and moved into place
/// (`MOVE`), so an interrupted upload never leaves a half object under its real name; a server
/// that cannot `MOVE` gets the direct upload, which is read back and checked by the caller.
pub struct DavStore {
    dav: WebDav,
    folder: Vec<String>,
    ensured: Mutex<HashSet<String>>,
    no_move: AtomicBool,
}

impl DavStore {
    /// `folder` is the path below the address, without [`REPO_DIR`].
    pub fn new(url: &str, user: &str, password: &str, folder: &str) -> Result<DavStore> {
        let mut parts: Vec<String> = folder
            .split('/')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        if parts.iter().any(|p| p == ".." || p == ".") {
            bail!("the folder name must not contain .. or .");
        }
        parts.push(REPO_DIR.to_string());
        Ok(DavStore {
            dav: WebDav::new(url, user, password)?,
            folder: parts,
            ensured: Mutex::new(HashSet::new()),
            no_move: AtomicBool::new(false),
        })
    }

    fn parts(&self, path: &str) -> Result<Vec<String>> {
        check_path(path)?;
        let mut v = self.folder.clone();
        v.extend(path.split('/').map(str::to_string));
        Ok(v)
    }

    fn ensure(&self, dir: &str) -> Result<()> {
        if self.ensured.lock().unwrap().contains(dir) {
            return Ok(());
        }
        let mut v = self.folder.clone();
        if !dir.is_empty() {
            v.extend(dir.split('/').map(str::to_string));
        }
        self.dav.ensure_dirs(&v)?;
        self.ensured.lock().unwrap().insert(dir.to_string());
        Ok(())
    }
}

struct CancelRead<'a> {
    inner: &'a mut dyn Read,
    cancel: &'a AtomicBool,
}

impl Read for CancelRead<'_> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("cancelled"));
        }
        self.inner.read(b)
    }
}

impl RepoStore for DavStore {
    fn label(&self) -> String {
        format!("{}{}", self.dav.base_url(), self.folder.join("/"))
    }

    fn check(&self) -> Result<()> {
        self.dav.check()?;
        self.dav.ensure_dirs(&self.folder)?;
        // does the server move files? (decides how objects are uploaded)
        let a = format!(".probe-{}", super::seal::random_hex(4)?);
        let b = format!("{a}-m");
        let mut pa = self.folder.clone();
        pa.push(a);
        let mut pb = self.folder.clone();
        pb.push(b);
        self.dav
            .put_stream(&pa, &mut std::io::Cursor::new(b"x"), 1)?;
        let moved = self.dav.move_file(&pa, &pb).unwrap_or(false);
        self.no_move.store(!moved, Ordering::Relaxed);
        let leftover = if moved { &pb } else { &pa };
        let _ = self
            .dav
            .delete_file(&leftover[..leftover.len() - 1], leftover.last().unwrap());
        Ok(())
    }

    fn free_space(&self) -> Option<u64> {
        self.dav.quota().map(|q| q.free)
    }

    fn list(&self, dir: &str) -> Result<Vec<Entry>> {
        check_dir(dir)?;
        let mut v = self.folder.clone();
        if !dir.is_empty() {
            v.extend(dir.split('/').map(str::to_string));
        }
        Ok(self
            .dav
            .list_dir(&v)?
            .into_iter()
            .filter_map(|e| {
                let name = e.rel.rsplit('/').next()?.to_string();
                Some(Entry {
                    name,
                    size: e.size,
                    is_dir: e.is_dir,
                })
            })
            .collect())
    }

    fn put(&self, path: &str, data: &mut dyn Read, len: u64, cancel: &AtomicBool) -> Result<()> {
        let fin = self.parts(path)?;
        let (dir, name) = split(path);
        self.ensure(dir)?;
        let direct = self.no_move.load(Ordering::Relaxed);
        let target = if direct {
            fin.clone()
        } else {
            let mut t = fin.clone();
            *t.last_mut().unwrap() = format!(".{name}.part-{}", super::seal::random_hex(4)?);
            t
        };
        let mut r = CancelRead {
            inner: data,
            cancel,
        };
        let res = self.dav.put_stream(&target, &mut r, len);
        let cleanup = |parts: &[String]| {
            let _ = self
                .dav
                .delete_file(&parts[..parts.len() - 1], parts.last().unwrap());
        };
        if let Err(e) = res {
            cleanup(&target);
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow::Error::new(Cancelled));
            }
            return Err(e);
        }
        if !direct {
            match self.dav.move_file(&target, &fin) {
                Ok(true) => {}
                Ok(false) => {
                    cleanup(&target);
                    bail!("the server stopped accepting MOVE; try again");
                }
                Err(e) => {
                    cleanup(&target);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn get_to(&self, path: &str, out: &mut dyn Write, max: u64) -> Result<u64> {
        self.dav.get_to(&self.parts(path)?, out, max)
    }

    fn delete(&self, path: &str) -> Result<()> {
        let p = self.parts(path)?;
        self.dav.delete_file(&p[..p.len() - 1], p.last().unwrap())
    }
}

// -- in memory (tests) ----------------------------------------------------------------------------------

/// A repository in memory that can be made unreachable, corrupt what it stores, run out of space
/// and be tampered with.
#[derive(Default)]
pub struct MemStore {
    pub files: Mutex<BTreeMap<String, Vec<u8>>>,
    pub down: Mutex<Option<String>>,
    /// Flip the last byte of every object written (tests verify-after-write).
    pub corrupt: Mutex<bool>,
    /// Pretend this many bytes are free.
    pub free: Mutex<Option<u64>>,
    /// Fail every `put` after this many succeeded.
    pub fail_after: Mutex<Option<usize>>,
    pub puts: Mutex<usize>,
}

impl MemStore {
    pub fn set_down(&self, why: Option<&str>) {
        *self.down.lock().unwrap() = why.map(str::to_string);
    }

    fn up(&self) -> Result<()> {
        match &*self.down.lock().unwrap() {
            Some(why) => Err(anyhow!("{why}")),
            None => Ok(()),
        }
    }

    /// Paths of everything stored, sorted.
    pub fn paths(&self) -> Vec<String> {
        self.files.lock().unwrap().keys().cloned().collect()
    }
}

impl RepoStore for MemStore {
    fn label(&self) -> String {
        "memory".into()
    }

    fn check(&self) -> Result<()> {
        self.up()
    }

    fn free_space(&self) -> Option<u64> {
        *self.free.lock().unwrap()
    }

    fn list(&self, dir: &str) -> Result<Vec<Entry>> {
        self.up()?;
        check_dir(dir)?;
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut seen = BTreeMap::new();
        for (p, d) in self.files.lock().unwrap().iter() {
            if let Some(rest) = p.strip_prefix(&prefix) {
                match rest.split_once('/') {
                    Some((sub, _)) => {
                        seen.insert(sub.to_string(), (0, true));
                    }
                    None => {
                        seen.insert(rest.to_string(), (d.len() as u64, false));
                    }
                }
            }
        }
        Ok(seen
            .into_iter()
            .map(|(name, (size, is_dir))| Entry { name, size, is_dir })
            .collect())
    }

    fn put(&self, path: &str, data: &mut dyn Read, len: u64, cancel: &AtomicBool) -> Result<()> {
        self.up()?;
        check_path(path)?;
        {
            let mut n = self.puts.lock().unwrap();
            if self.fail_after.lock().unwrap().is_some_and(|k| *n >= k) {
                bail!("the place refused the upload");
            }
            *n += 1;
        }
        let mut v = Vec::new();
        copy_exact(data, &mut v, len, cancel)?;
        if *self.corrupt.lock().unwrap() {
            if let Some(b) = v.last_mut() {
                *b ^= 0xff;
            }
        }
        self.files.lock().unwrap().insert(path.to_string(), v);
        Ok(())
    }

    fn get_to(&self, path: &str, out: &mut dyn Write, max: u64) -> Result<u64> {
        self.up()?;
        check_path(path)?;
        let d = self
            .files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow!("{path} is not there"))?;
        copy_limited(&mut std::io::Cursor::new(d), out, max)
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.up()?;
        check_path(path)?;
        self.files.lock().unwrap().remove(path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("bookx-rs-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn only_repository_paths_are_accepted() {
        for ok in [
            "bookx-repo.json",
            "blobs/ab/abcdef",
            "snapshots/s1.snap",
            "journal/j1",
        ] {
            assert!(valid_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "blobs/../x",
            "a/b/c/d",
            "blobs//x",
            "a b",
            "a\\b",
            "x\0y",
            "blobs/ab/..",
            "é",
        ] {
            assert!(!valid_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_folder_store_puts_atomically_lists_gets_and_deletes() {
        let d = tmp();
        std::fs::create_dir_all(&d).unwrap();
        let s = FolderStore::new(d.join(REPO_DIR));
        s.check().unwrap();
        let no = AtomicBool::new(false);
        s.put_bytes("blobs/ab/x1", b"hello", &no).unwrap();
        assert_eq!(s.stat("blobs/ab/x1").unwrap(), Some(5));
        assert_eq!(s.stat("blobs/ab/zz").unwrap(), None);
        assert_eq!(s.get_bytes("blobs/ab/x1", 10).unwrap(), b"hello");
        assert!(s.get_bytes("blobs/ab/x1", 4).is_err(), "bounded");
        assert_eq!(s.list("blobs").unwrap()[0].name, "ab");
        // wrong length: nothing visible, nothing left behind
        assert!(s
            .put("blobs/ab/x2", &mut std::io::Cursor::new(b"abc"), 5, &no)
            .is_err());
        assert!(s
            .put("blobs/ab/x2", &mut std::io::Cursor::new(b"abcdef"), 5, &no)
            .is_err());
        let names: Vec<String> = s
            .list("blobs/ab")
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["x1"], "{names:?}");
        // cancel before the first block
        let yes = AtomicBool::new(true);
        let e = s.put_bytes("blobs/ab/x3", b"zzz", &yes).unwrap_err();
        assert!(e.downcast_ref::<Cancelled>().is_some());
        assert_eq!(s.list("blobs/ab").unwrap().len(), 1);
        s.delete("blobs/ab/x1").unwrap();
        s.delete("blobs/ab/x1").unwrap();
        assert!(s.delete("../x").is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn links_in_a_hostile_repository_folder_are_not_followed() {
        let d = tmp();
        std::fs::create_dir_all(d.join("R/blobs")).unwrap();
        std::fs::write(d.join("secret"), "top secret").unwrap();
        std::os::unix::fs::symlink(d.join("secret"), d.join("R/blobs/leak")).unwrap();
        std::os::unix::fs::symlink(d.join("elsewhere"), d.join("R/journal")).unwrap();
        std::fs::create_dir_all(d.join("elsewhere")).unwrap();
        let s = FolderStore::new(d.join("R"));
        assert!(s.get_bytes("blobs/leak", 100).is_err());
        assert!(s.list("blobs").unwrap().is_empty(), "links are not listed");
        assert!(s
            .put_bytes("journal/j1", b"x", &AtomicBool::new(false))
            .is_err());
        assert!(!d.join("elsewhere/j1").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_missing_drive_is_not_created_out_of_thin_air() {
        let s = FolderStore::new("/definitely-not-mounted/Bookx Backup");
        let e = s.check().unwrap_err().to_string();
        assert!(e.contains("plugged in") || e.contains("mounted"), "{e}");
    }

    #[test]
    fn the_memory_store_lists_like_a_folder() {
        let m = MemStore::default();
        let no = AtomicBool::new(false);
        m.put_bytes("blobs/aa/1", b"1", &no).unwrap();
        m.put_bytes("blobs/bb/2", b"22", &no).unwrap();
        m.put_bytes("bookx-repo.json", b"{}", &no).unwrap();
        let top = m.list("").unwrap();
        assert!(top.iter().any(|e| e.name == "blobs" && e.is_dir));
        assert!(top.iter().any(|e| e.name == "bookx-repo.json" && !e.is_dir));
        assert_eq!(m.list("blobs/bb").unwrap()[0].size, 2);
    }
}
