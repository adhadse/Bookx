//! The library being backed up: its catalogue, the notes Bookx manages for each book, the walk
//! over its files (what is in scope and what is not), and the small state kept next to the library
//! so unchanged files are not read again.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bookx_core::library::Book;
use bookx_store::{ReadingStatus, SqliteStore, LIBRARY_DB};
use serde::{Deserialize, Serialize};

use super::manifest::{
    validate_rel, BookNotes, CollectionState, LibraryState, Role, FORMAT, MAX_FILES,
};

/// Where the local backup state lives, inside the library's own state folder.
pub const LOCAL_DIR: &str = ".bookx/backup";
/// Temporary copies made during a run.
pub const TMP_DIR: &str = ".bookx/backup/tmp";

/// A library to back up: its folder and its open catalogue.
#[derive(Clone)]
pub struct LibrarySource {
    pub root: PathBuf,
    pub store: Arc<SqliteStore>,
}

impl LibrarySource {
    /// Opens the catalogue of the library in `root` (it must have one).
    pub fn open(root: &Path) -> Result<LibrarySource> {
        let db = root.join(LIBRARY_DB);
        if !db.exists() {
            bail!("there is no Bookx library in {}", root.display());
        }
        Ok(LibrarySource {
            root: root.to_path_buf(),
            store: Arc::new(SqliteStore::open(&db)?),
        })
    }

    /// For the app, which has the catalogue open already.
    pub fn with_store(root: &Path, store: Arc<SqliteStore>) -> LibrarySource {
        LibrarySource {
            root: root.to_path_buf(),
            store,
        }
    }

    pub fn books(&self) -> Result<Vec<Book>> {
        self.store.all_books()
    }

    /// The notes of one book.
    pub fn notes_of(book: &Book, status: Option<&str>) -> BookNotes {
        BookNotes {
            id: book.id,
            file: book.file.clone(),
            title: book.meta.title.clone(),
            highlights: book.highlights.clone(),
            bookmarks: book.bookmarks.clone(),
            position: book.position,
            progress: book.progress,
            last_read: book.last_read,
            status: status.map(str::to_string),
        }
    }

    fn statuses(&self) -> HashMap<u64, &'static str> {
        let mut m = HashMap::new();
        for s in [
            ReadingStatus::Reading,
            ReadingStatus::OnHold,
            ReadingStatus::Finished,
        ] {
            for id in self.store.books_with_status(s) {
                m.insert(id, s.as_str());
            }
        }
        m
    }

    /// The notes of every book that has any, in id order.
    pub fn all_notes(&self, books: &[Book]) -> Vec<BookNotes> {
        let st = self.statuses();
        let mut v: Vec<BookNotes> = books
            .iter()
            .map(|b| Self::notes_of(b, st.get(&b.id).copied()))
            .filter(|n| !n.is_empty())
            .collect();
        v.sort_by_key(|n| n.id);
        v
    }

    /// The readable notes state stored with every snapshot.
    pub fn library_state(&self, books: &[Book]) -> Result<LibraryState> {
        Ok(LibraryState {
            format: FORMAT,
            books: self.all_notes(books),
            collections: self.collections()?,
        })
    }

    /// The collections (shelves) with their book ids, sorted by name.
    pub fn collections(&self) -> Result<Vec<CollectionState>> {
        let mut collections = Vec::new();
        for c in self.store.list_collections()? {
            let mut ids = self.store.collection_books(c.id);
            ids.sort_unstable();
            collections.push(CollectionState {
                name: c.name,
                books: ids,
            });
        }
        collections.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(collections)
    }

    pub fn tmp_dir(&self) -> Result<PathBuf> {
        let d = self.root.join(TMP_DIR);
        std::fs::create_dir_all(&d).with_context(|| format!("cannot create {}", d.display()))?;
        Ok(d)
    }
}

// -- the walk --------------------------------------------------------------------------------------------

/// What the walk includes.
#[derive(Debug, Clone)]
pub struct Scope {
    pub include_books: bool,
    pub include_covers: bool,
    /// Files bigger than this are left out and reported.
    pub max_file_size: Option<u64>,
}

impl Default for Scope {
    fn default() -> Self {
        Scope {
            include_books: true,
            include_covers: true,
            max_file_size: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub rel: String,
    pub abs: PathBuf,
    pub size: u64,
    pub mtime: i64,
    pub mtime_ns: i128,
    pub role: Role,
}

/// A file the walk saw and left out, with the reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub files: Vec<ScannedFile>,
    pub skipped: Vec<Skipped>,
    pub too_big: Vec<String>,
    /// Folders and files that are never part of a backup (hidden, caches, partial downloads…).
    pub excluded: usize,
}

const IMAGE_EXT: [&str; 5] = ["jpg", "jpeg", "png", "webp", "gif"];

/// Names that are never backed up: partial downloads, temporary files, lock files, the catalogue
/// itself (it goes in as a consistent copy) and OS litter.
pub fn is_excluded_name(name: &str) -> bool {
    let l = name.to_lowercase();
    name.starts_with('.')
        || l.starts_with("bookx-library.")
        || l.ends_with(".part")
        || l.ends_with(".partial")
        || l.ends_with(".crdownload")
        || l.ends_with(".tmp")
        || l.ends_with(".temp")
        || l.ends_with(".lock")
        || l.ends_with('~')
        || l.starts_with("~$")
        || l == "thumbs.db"
        || l == "desktop.ini"
}

fn is_cover(rel: &str, covers: &HashSet<&str>) -> bool {
    if covers.contains(rel) {
        return true;
    }
    let name = rel.rsplit('/').next().unwrap_or(rel).to_lowercase();
    match name.rsplit_once('.') {
        Some((stem, ext)) => stem == "cover" && IMAGE_EXT.contains(&ext),
        None => false,
    }
}

/// Walks the library folder. Symbolic links are never followed; hidden folders (`.bookx`,
/// `.bookx-backups`, `.bookx-trash`) and the names of [`is_excluded_name`] are left out; names that
/// a snapshot could not carry safely are reported, not silently dropped.
pub fn scan_library(root: &Path, books: &[Book], scope: &Scope) -> Result<Scan> {
    let mut scan = Scan::default();
    if !scope.include_books && !scope.include_covers {
        return Ok(scan);
    }
    let covers: HashSet<&str> = books.iter().filter_map(|b| b.cover.as_deref()).collect();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) => {
                scan.skipped.push(Skipped {
                    path: rel.clone(),
                    reason: format!("cannot read the folder: {e}"),
                });
                continue;
            }
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() || is_excluded_name(&name) {
                scan.excluded += 1;
                continue;
            }
            if ft.is_dir() {
                // a backup repository inside the library (a place chosen below it) is never backed up into itself
                if e.path().join("bookx-repo.json").is_file() {
                    scan.excluded += 1;
                    continue;
                }
                stack.push((e.path(), child));
                continue;
            }
            if !ft.is_file() {
                scan.excluded += 1; // devices, sockets, pipes
                continue;
            }
            if e.file_name().to_str().is_none() {
                scan.skipped.push(Skipped {
                    path: child,
                    reason: "the name is not valid text".into(),
                });
                continue;
            }
            if let Err(why) = validate_rel(&child) {
                scan.skipped.push(Skipped {
                    path: child,
                    reason: format!("{why}"),
                });
                continue;
            }
            let role = if is_cover(&child, &covers) {
                Role::Cover
            } else {
                Role::Book
            };
            if (role == Role::Book && !scope.include_books)
                || (role == Role::Cover && !scope.include_covers)
            {
                scan.excluded += 1;
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            if scope.max_file_size.is_some_and(|max| m.len() > max) {
                scan.too_big.push(child);
                continue;
            }
            let mtime_ns = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i128)
                .unwrap_or(0);
            scan.files.push(ScannedFile {
                rel: child,
                abs: e.path(),
                size: m.len(),
                mtime: (mtime_ns / 1_000_000_000) as i64,
                mtime_ns,
                role,
            });
            if scan.files.len() > MAX_FILES {
                bail!("the library has more than {MAX_FILES} files; too many for one backup");
            }
        }
    }
    scan.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    scan.too_big.sort();
    Ok(scan)
}

// -- local state -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CachedFile {
    pub size: u64,
    pub mtime_ns: i128,
    pub blob: String,
}

/// What the library remembers about one repository: hashes of files already seen (so unchanged
/// files are not read again), the notes already journaled, the newest snapshot ever seen (to notice
/// a place that went back in time).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LocalState {
    pub format: u32,
    pub repo: String,
    /// Which keys the cached ids belong to.
    pub key_fingerprint: String,
    pub last_journal_seq: u64,
    pub last_seen_seq: u64,
    pub notes: BTreeMap<u64, String>,
    /// Fingerprint of the collections last written to the journal (or to a snapshot).
    pub collections: String,
    pub files: HashMap<String, CachedFile>,
}

impl LocalState {
    fn path(root: &Path, repo: &str) -> PathBuf {
        root.join(LOCAL_DIR).join(format!("{repo}.json"))
    }

    /// Loads the state for `repo`; a missing, damaged or foreign one is a fresh, empty state.
    pub fn load(root: &Path, repo: &str, key_fingerprint: &str) -> LocalState {
        let fresh = || LocalState {
            format: FORMAT,
            repo: repo.to_string(),
            key_fingerprint: key_fingerprint.to_string(),
            ..LocalState::default()
        };
        let Ok(text) = std::fs::read(Self::path(root, repo)) else {
            return fresh();
        };
        match serde_json::from_slice::<LocalState>(&text) {
            Ok(s) if s.repo == repo => {
                if s.key_fingerprint == key_fingerprint {
                    s
                } else {
                    // other keys: ids differ, so the cache is useless, but what was seen stays true
                    LocalState {
                        files: HashMap::new(),
                        notes: BTreeMap::new(),
                        last_journal_seq: s.last_journal_seq,
                        last_seen_seq: s.last_seen_seq,
                        ..fresh()
                    }
                }
            }
            _ => fresh(),
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let p = Self::path(root, &self.repo);
        std::fs::create_dir_all(p.parent().expect("has a parent"))?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }

    /// The cached id of `f`, when its size and modification time are as they were.
    pub fn cached(&self, f: &ScannedFile) -> Option<&str> {
        self.files
            .get(&f.rel)
            .filter(|c| c.size == f.size && c.mtime_ns == f.mtime_ns)
            .map(|c| c.blob.as_str())
    }
}

/// The newest snapshot number this library has seen at `repo` (0 when none), for the rollback check.
pub fn last_seen_seq(root: &Path, repo: &str) -> u64 {
    LocalState::load(root, repo, "").last_seen_seq
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("bookx-src-{}", uuid::Uuid::new_v4()))
    }

    fn w(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn the_walk_keeps_books_and_covers_and_leaves_state_trash_and_partials_out() {
        let d = tmp();
        w(&d, "Ann/One/one.epub", "1");
        w(&d, "Ann/One/cover.jpg", "c");
        w(&d, "Ann/Two/two.epub", "22");
        w(&d, ".bookx/oplog.json", "x");
        w(&d, ".bookx-trash/gone.epub", "x");
        w(&d, ".bookx-backups/bookx-1.db", "x");
        w(&d, "Ann/One/download.epub.part", "x");
        w(&d, "bookx-library.db", "x");
        w(&d, "bookx-library.db-wal", "x");
        w(&d, "con.epub", "x");
        w(&d, "Backups/Bookx Backup/bookx-repo.json", "{}");
        w(&d, "Backups/Bookx Backup/blobs/ab/abcd", "x");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", d.join("link.epub")).unwrap();
        let s = scan_library(&d, &[], &Scope::default()).unwrap();
        let names: Vec<&str> = s.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(
            names,
            vec!["Ann/One/cover.jpg", "Ann/One/one.epub", "Ann/Two/two.epub"]
        );
        assert_eq!(s.files[0].role, Role::Cover);
        assert_eq!(s.skipped.len(), 1, "{:?}", s.skipped);
        assert!(s.skipped[0].path == "con.epub" && s.skipped[0].reason.contains("reserved"));

        let no_cov = scan_library(
            &d,
            &[],
            &Scope {
                include_covers: false,
                ..Scope::default()
            },
        )
        .unwrap();
        assert_eq!(no_cov.files.len(), 2);
        let big = scan_library(
            &d,
            &[],
            &Scope {
                max_file_size: Some(1),
                ..Scope::default()
            },
        )
        .unwrap();
        assert_eq!(big.too_big, vec!["Ann/Two/two.epub"]);
        let none = scan_library(
            &d,
            &[],
            &Scope {
                include_books: false,
                include_covers: false,
                ..Scope::default()
            },
        )
        .unwrap();
        assert!(none.files.is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_cache_only_trusts_an_unchanged_file_and_the_same_keys() {
        let d = tmp();
        let mut st = LocalState::load(&d, "repo1", "k1");
        let f = ScannedFile {
            rel: "a.epub".into(),
            abs: d.join("a.epub"),
            size: 3,
            mtime: 5,
            mtime_ns: 5_000_000_000,
            role: Role::Book,
        };
        st.files.insert(
            "a.epub".into(),
            CachedFile {
                size: 3,
                mtime_ns: 5_000_000_000,
                blob: "b".repeat(64),
            },
        );
        st.last_seen_seq = 4;
        st.save(&d).unwrap();
        let again = LocalState::load(&d, "repo1", "k1");
        assert_eq!(again.cached(&f), Some("b".repeat(64).as_str()));
        let mut changed = f.clone();
        changed.mtime_ns += 1;
        assert!(again.cached(&changed).is_none());
        let other_keys = LocalState::load(&d, "repo1", "k2");
        assert!(other_keys.cached(&f).is_none());
        assert_eq!(other_keys.last_seen_seq, 4, "what was seen is kept");
        assert!(LocalState::load(&d, "repo2", "k1").files.is_empty());
        let _ = std::fs::remove_dir_all(d);
    }
}
