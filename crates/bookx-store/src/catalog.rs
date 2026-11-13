//! The library catalogue in SQLite: [`open_library`] gives a [`Library`] whose books are kept in
//! `<root>/bookx-library.db`, and moves an old `bookx-library.json` into it once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use bookx_core::library::{Book, CatalogBackend, Library};

use crate::store::SqliteStore;

/// The catalogue file inside a library folder.
pub const LIBRARY_DB: &str = "bookx-library.db";
/// The catalogue of earlier versions. Read once, then renamed to `bookx-library.json.migrated`.
pub const LEGACY_JSON: &str = "bookx-library.json";

pub fn library_db_path(root: &Path) -> PathBuf {
    root.join(LIBRARY_DB)
}

/// True when `root` holds a library (a database, or an old catalogue waiting to be migrated).
pub fn has_library(root: &Path) -> bool {
    library_db_path(root).exists() || root.join(LEGACY_JSON).exists()
}

/// The [`CatalogBackend`] over a [`SqliteStore`].
pub struct SqliteCatalog {
    store: Arc<SqliteStore>,
}

impl SqliteCatalog {
    pub fn new(store: Arc<SqliteStore>) -> SqliteCatalog {
        SqliteCatalog { store }
    }
}

impl CatalogBackend for SqliteCatalog {
    fn load(&mut self) -> Result<(u64, Vec<Book>)> {
        Ok((self.store.next_id(), self.store.all_books()?))
    }

    fn commit(&mut self, next_id: u64, upserts: &[&Book], removed: &[u64]) -> Result<()> {
        self.store.commit_catalog(next_id, upserts, removed)
    }
}

/// An opened library with the store behind it (shared with the full-text index).
pub struct OpenedLibrary {
    pub library: Library,
    pub store: Arc<SqliteStore>,
    /// Number of books moved out of an old `bookx-library.json`, when that just happened.
    pub migrated: Option<usize>,
}

/// Opens (creating if needed) the library in `root`.
///
/// An old `bookx-library.json` is imported when the database is new, then renamed to
/// `bookx-library.json.migrated` so it is never read again. A damaged old catalogue is an error
/// that names the file, and nothing is lost: the file stays where it is.
pub fn open_library(root: &Path) -> Result<OpenedLibrary> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("cannot create the library folder {}", root.display()))?;
    let store = Arc::new(SqliteStore::open(&library_db_path(root))?);
    let migrated = migrate_legacy(root, &store)?;
    let library = Library::with_backend(root, Box::new(SqliteCatalog::new(store.clone())))?;
    Ok(OpenedLibrary {
        library,
        store,
        migrated,
    })
}

fn migrate_legacy(root: &Path, store: &SqliteStore) -> Result<Option<usize>> {
    let json = root.join(LEGACY_JSON);
    if !json.exists() || !store.is_empty() || store.next_id() > 1 {
        return Ok(None);
    }
    let report = store.import_json_catalog(&json).with_context(|| {
        format!(
            "the old catalogue {} could not be read; move it away or fix it and open the library again",
            json.display()
        )
    })?;
    let mut target = root.join(format!("{LEGACY_JSON}.migrated"));
    let mut n = 1;
    while target.exists() {
        n += 1;
        target = root.join(format!("{LEGACY_JSON}.migrated{n}"));
    }
    std::fs::rename(&json, &target)
        .with_context(|| format!("cannot move {} out of the way", json.display()))?;
    Ok(Some(report.imported))
}

/// Follows a library's database from another connection (the sharing server): the books are
/// re-read only when something was committed.
pub struct CatalogWatcher {
    db: PathBuf,
    state: Mutex<WatchState>,
}

#[derive(Default)]
struct WatchState {
    store: Option<SqliteStore>,
    version: Option<i64>,
    books: Vec<Book>,
}

impl CatalogWatcher {
    pub fn new(root: &Path) -> CatalogWatcher {
        CatalogWatcher {
            db: library_db_path(root),
            state: Mutex::new(WatchState::default()),
        }
    }

    /// The current books. An unreadable or missing database gives the last known list (empty at first).
    pub fn books(&self) -> Vec<Book> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.store.is_none() {
            if !self.db.exists() {
                return st.books.clone();
            }
            match SqliteStore::open(&self.db) {
                Ok(s) => st.store = Some(s),
                Err(_) => return st.books.clone(),
            }
        }
        let current = st.store.as_ref().map(|s| s.data_version());
        if st.version.is_none() || st.version != current {
            if let Some(books) = st.store.as_ref().and_then(|s| s.all_books().ok()) {
                st.books = books;
                st.version = current;
            }
        }
        st.books.clone()
    }
}
