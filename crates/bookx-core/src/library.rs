//! The on-disk library: book files under a folder layout of the user's choice (see
//! [`crate::layout`]) plus a catalogue kept by a [`CatalogBackend`] (SQLite in `bookx-store`).
//!
//! ```text
//! <root>/bookx-library.db
//! <root>/Author Name/Book Title (12)/Book Title - Author Name.epub   (the default Tree layout)
//! <root>/Author Name/Book Title (12)/cover.jpg
//! ```
//!
//! File operations are transactional ([`crate::fsops`]), deleted books go to a trash and the
//! operations are logged for undo ([`crate::history`], the `impl Library` in `library_files.rs`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::archive::Archive;
use super::book::OpenBook;
use super::convert;
use super::layout::{self, Layout, TemplateFn};
use super::opf::Metadata;
use super::query::Query;
use super::util::*;

/// A durable home for the catalogue. `bookx-store` implements it with SQLite; the library keeps
/// the books in memory and hands the backend only what changed.
pub trait CatalogBackend: Send {
    /// `(next free id, all books)`.
    fn load(&mut self) -> Result<(u64, Vec<Book>)>;
    /// Atomically stores `upserts`, deletes `removed` and records `next_id`.
    fn commit(&mut self, next_id: u64, upserts: &[&Book], removed: &[u64]) -> Result<()>;
    /// The custom column definitions (empty for a backend that cannot keep them).
    fn load_columns(&mut self) -> Result<Vec<crate::custom::ColumnDef>> {
        Ok(Vec::new())
    }
    /// Replaces the custom column definitions.
    fn save_columns(&mut self, _columns: &[crate::custom::ColumnDef]) -> Result<()> {
        Ok(())
    }
}

/// A backend that keeps everything in memory. Clones share the data, so it can stand in for a
/// database in tests ("reopening" a library with a clone sees what was saved).
#[derive(Clone, Default)]
pub struct MemoryBackend {
    data: std::sync::Arc<std::sync::Mutex<(u64, Vec<Book>)>>,
    columns: std::sync::Arc<std::sync::Mutex<Vec<crate::custom::ColumnDef>>>,
}

impl MemoryBackend {
    pub fn new() -> MemoryBackend {
        MemoryBackend::default()
    }

    /// Number of commits that stored or removed something (tests).
    pub fn books(&self) -> usize {
        self.lock().1.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (u64, Vec<Book>)> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl CatalogBackend for MemoryBackend {
    fn load_columns(&mut self) -> Result<Vec<crate::custom::ColumnDef>> {
        Ok(self
            .columns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone())
    }

    fn save_columns(&mut self, columns: &[crate::custom::ColumnDef]) -> Result<()> {
        *self.columns.lock().unwrap_or_else(|e| e.into_inner()) = columns.to_vec();
        Ok(())
    }

    fn load(&mut self) -> Result<(u64, Vec<Book>)> {
        let d = self.lock();
        Ok((d.0, d.1.clone()))
    }

    fn commit(&mut self, next_id: u64, upserts: &[&Book], removed: &[u64]) -> Result<()> {
        let mut d = self.lock();
        d.0 = next_id;
        d.1.retain(|b| !removed.contains(&b.id) && !upserts.iter().any(|u| u.id == b.id));
        d.1.extend(upserts.iter().map(|b| (*b).clone()));
        d.1.sort_by_key(|b| b.id);
        Ok(())
    }
}

/// A fingerprint of the book's catalogue record; `buf` is reused between books.
fn book_hash_with(buf: &mut Vec<u8>, b: &Book) -> u64 {
    use std::hash::Hasher;
    buf.clear();
    let _ = serde_json::to_writer(&mut *buf, b);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write(buf);
    h.finish()
}

/// The hashes of all `books`, in order. Serialising 100,000 books takes about 100 ms on one core, so
/// large libraries are hashed on several threads (`save` runs on every page turn).
fn book_hashes(books: &[Book]) -> Vec<u64> {
    const PARALLEL_FROM: usize = 4096;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8);
    if books.len() < PARALLEL_FROM || threads < 2 {
        let mut buf = Vec::new();
        return books.iter().map(|b| book_hash_with(&mut buf, b)).collect();
    }
    let chunk = books.len().div_ceil(threads);
    let mut parts: Vec<Option<Vec<u64>>> = Vec::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = books
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    let mut buf = Vec::new();
                    c.iter()
                        .map(|b| book_hash_with(&mut buf, b))
                        .collect::<Vec<u64>>()
                })
            })
            .collect();
        for h in handles {
            parts.push(h.join().ok());
        }
    });
    match parts.into_iter().collect::<Option<Vec<_>>>() {
        Some(p) => p.concat(),
        None => {
            let mut buf = Vec::new();
            books.iter().map(|b| book_hash_with(&mut buf, b)).collect()
        }
    }
}

/// What the backend already has.
#[derive(Default)]
struct Saved {
    hashes: std::collections::HashMap<u64, u64>,
    next_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Highlight {
    pub id: String,
    pub spine: usize,
    pub start: i32,
    pub end: i32,
    pub color: String,
    /// `background` (default, also for old catalogues), `underline`, `strikethrough`, `squiggly`, `invert`.
    #[serde(default)]
    pub style: String,
    pub note: String,
    pub text: String,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bookmark {
    pub id: String,
    pub spine: usize,
    pub frac: f64,
    pub label: String,
    pub created: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct ReadPos {
    pub spine: usize,
    pub frac: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: u64,
    /// Path of the book file relative to the library root.
    pub file: String,
    #[serde(default)]
    pub cover: Option<String>,
    pub meta: Metadata,
    pub added: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    #[serde(default)]
    pub last_read: Option<DateTime<Utc>>,
    pub size: u64,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub position: Option<ReadPos>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    #[serde(default)]
    pub highlights: Vec<Highlight>,
    /// Further format files of the same book (the primary one is `file`).
    #[serde(default)]
    pub formats: Vec<crate::formats::FormatFile>,
    /// Linked resources that are not book files, such as an audiobook folder.
    #[serde(default)]
    pub resources: Vec<crate::formats::Resource>,
    /// Copy protection found at import (`Kindle`, `Adobe ADEPT`, ...); empty when none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub drm: String,
    /// Values of the custom columns by label, as [`crate::custom::ColumnDef::normalize`] stores them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom: BTreeMap<String, String>,
    /// Temporarily marked in this session (search `marked:true`); never saved.
    #[serde(skip)]
    pub marked: bool,
}

impl Book {
    pub fn format(&self) -> &'static str {
        "epub"
    }

    /// Every format file of the book, the primary one first.
    pub fn all_formats(&self) -> Vec<crate::formats::FormatFile> {
        let mut v = vec![crate::formats::FormatFile {
            file: self.file.clone(),
            format: crate::formats::format_of(&self.file),
            size: self.size,
        }];
        v.extend(self.formats.iter().cloned());
        v
    }

    /// The file (relative to the library, or absolute when linked) that suits `target` best.
    pub fn file_for(&self, target: crate::formats::Target) -> String {
        let all = self.all_formats();
        crate::formats::choose(&all, target)
            .map(|f| f.file.clone())
            .unwrap_or_else(|| self.file.clone())
    }

    pub fn status(&self) -> &'static str {
        if self.progress >= 0.98 {
            "finished"
        } else if self.progress > 0.0 || self.position.is_some() {
            "reading"
        } else {
            "unread"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortKey {
    #[default]
    Title,
    Author,
    DateAdded,
    LastRead,
    Published,
    Rating,
    Series,
    Size,
    Progress,
}

impl SortKey {
    pub const ALL: [SortKey; 9] = [
        Self::Title,
        Self::Author,
        Self::DateAdded,
        Self::LastRead,
        Self::Published,
        Self::Rating,
        Self::Series,
        Self::Size,
        Self::Progress,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Author => "Author",
            Self::DateAdded => "Date added",
            Self::LastRead => "Last read",
            Self::Published => "Published",
            Self::Rating => "Rating",
            Self::Series => "Series",
            Self::Size => "Size",
            Self::Progress => "Progress",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Authors,
    Series,
    Tags,
    Publishers,
    Languages,
    Ratings,
    Status,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Self::Authors,
        Self::Series,
        Self::Tags,
        Self::Publishers,
        Self::Languages,
        Self::Ratings,
        Self::Status,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Authors => "Authors",
            Self::Series => "Series",
            Self::Tags => "Tags",
            Self::Publishers => "Publishers",
            Self::Languages => "Languages",
            Self::Ratings => "Ratings",
            Self::Status => "Reading status",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Authors => "avatar-default-symbolic",
            Self::Series => "view-list-symbolic",
            Self::Tags => "tag-symbolic",
            Self::Publishers => "publisher-symbolic",
            Self::Languages => "preferences-desktop-locale-symbolic",
            Self::Ratings => "starred-symbolic",
            Self::Status => "document-open-recent-symbolic",
        }
    }

    /// The search expression that selects `item` within this category.
    pub fn search_for(&self, item: &str) -> String {
        let quoted = format!("\"{item}\"");
        match self {
            Self::Authors => format!("author:={quoted}"),
            Self::Series => format!("series:={quoted}"),
            Self::Tags => format!("tag:={quoted}"),
            Self::Publishers => format!("publisher:={quoted}"),
            Self::Languages => format!("language:={quoted}"),
            Self::Ratings => format!("rating:{}", item.chars().filter(|c| *c == '★').count()),
            Self::Status => format!("status:{item}"),
        }
    }
}

#[derive(Debug)]
pub struct Prepared {
    pub meta: Metadata,
    pub cover: Option<(Vec<u8>, String)>,
    pub bytes: Vec<u8>,
    /// The EPUB file the book was read from, when it was not converted. A library with the
    /// `Linked` layout keeps the book there instead of copying it.
    pub origin: Option<PathBuf>,
    /// Copy protection that was detected (the label of [`crate::drm::DrmKind`]); empty when none.
    pub drm: String,
    /// The file the book was read from, whatever its format (set by the importer).
    pub source_file: Option<PathBuf>,
}

#[allow(dead_code)]
pub enum AddOutcome {
    Added(u64),
    Duplicate(u64),
}

pub struct Library {
    pub root: PathBuf,
    pub(crate) next_id: u64,
    pub books: Vec<Book>,
    backend: std::sync::Mutex<Box<dyn CatalogBackend>>,
    saved: std::sync::Mutex<Saved>,
    pub(crate) layout: Layout,
    pub(crate) template_fn: Option<TemplateFn>,
    pub(crate) read_only: bool,
    pub(crate) columns: Vec<crate::custom::ColumnDef>,
    pub(crate) ops: Vec<crate::history::Op>,
    /// True while an undo runs, so it is not logged as an operation of its own.
    pub(crate) replaying: bool,
}

/// Reads a book file and gathers everything needed to add it (thread-safe, no library state).
pub fn prepare_import(path: &Path) -> Result<Prepared> {
    let archive = if convert::is_importable(path) {
        convert::import_to_epub(path)?
    } else {
        Archive::open(path)?
    };
    let mut p = prepare_from_archive(archive, path)?;
    if !convert::is_importable(path) {
        p.origin = Some(path.to_path_buf());
        p.drm = crate::drm::detect(path)
            .map(|d| d.kind.label().to_string())
            .unwrap_or_default();
    }
    Ok(p)
}

/// Gathers everything needed to add a book that was already decoded to an EPUB archive.
/// `origin` only supplies a fallback title (its file name).
pub fn prepare_from_archive(archive: Archive, origin: &Path) -> Result<Prepared> {
    let path = origin;
    let book = OpenBook::from_archive(archive)?;
    let mut meta = book.metadata().clone();
    if meta.title.trim().is_empty() {
        meta.title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();
    }
    if meta.authors.is_empty() {
        meta.authors = vec!["Unknown".to_string()];
        meta.author_sort = "Unknown".into();
    }
    let cover = book.package.cover_path().and_then(|p| {
        book.archive
            .get(&p)
            .map(|b| (b.to_vec(), extension(&p).replace("jpeg", "jpg")))
    });
    Ok(Prepared {
        meta,
        cover,
        bytes: book.archive.to_bytes()?,
        origin: None,
        drm: String::new(),
        source_file: None,
    })
}

pub fn collect_book_files(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            if recursive {
                collect_book_files(&p, true, out);
            }
        } else if is_supported_file(&p) {
            out.push(p);
        }
    }
}

pub fn is_supported_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("epub"))
        .unwrap_or(false)
        || convert::is_importable(p)
}

impl Library {
    /// Opens the library in `root` with its catalogue in `backend`.
    pub fn with_backend(root: &Path, mut backend: Box<dyn CatalogBackend>) -> Result<Library> {
        std::fs::create_dir_all(root)?;
        let (next_id, books) = backend.load().context("reading the library catalogue")?;
        let columns = backend
            .load_columns()
            .context("reading the custom columns")?;
        let next_id = next_id
            .max(books.iter().map(|b| b.id + 1).max().unwrap_or(1))
            .max(1);
        let saved = Saved {
            hashes: books
                .iter()
                .zip(book_hashes(&books))
                .map(|(b, h)| (b.id, h))
                .collect(),
            next_id,
        };
        Ok(Library {
            root: root.to_path_buf(),
            next_id,
            books,
            backend: std::sync::Mutex::new(backend),
            saved: std::sync::Mutex::new(saved),
            layout: layout::load_layout(root),
            template_fn: None,
            read_only: false,
            columns,
            ops: crate::history::load_ops(root),
            replaying: false,
        })
    }

    pub(crate) fn backend_save_columns(&self, defs: &[crate::custom::ColumnDef]) -> Result<()> {
        self.backend
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .save_columns(defs)
    }

    /// A library whose catalogue lives only in memory (tests).
    pub fn in_memory(root: &Path) -> Result<Library> {
        Library::with_backend(root, Box::new(MemoryBackend::new()))
    }

    /// Stores what changed since the last save: new and edited books, removed books, the id counter.
    /// Cheap when nothing changed, and proportional to the changes (not the library size) in the
    /// database.
    pub fn save(&self) -> Result<()> {
        if self.read_only {
            return Ok(());
        }
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        let mut changed: Vec<(&Book, u64)> = Vec::new();
        let mut present = std::collections::HashSet::with_capacity(self.books.len());
        let hashes = book_hashes(&self.books);
        for (b, h) in self.books.iter().zip(hashes) {
            present.insert(b.id);
            if saved.hashes.get(&b.id) != Some(&h) {
                changed.push((b, h));
            }
        }
        let removed: Vec<u64> = saved
            .hashes
            .keys()
            .copied()
            .filter(|id| !present.contains(id))
            .collect();
        if changed.is_empty() && removed.is_empty() && saved.next_id == self.next_id {
            return Ok(());
        }
        let upserts: Vec<&Book> = changed.iter().map(|(b, _)| *b).collect();
        self.backend
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .commit(self.next_id, &upserts, &removed)?;
        for id in &removed {
            saved.hashes.remove(id);
        }
        for (b, h) in changed {
            saved.hashes.insert(b.id, h);
        }
        saved.next_id = self.next_id;
        Ok(())
    }

    /// The books for `ids`, in that order (unknown ids are left out). One pass over the library:
    /// calling [`get`](Self::get) per id is quadratic, which a 100,000 book grid cannot afford.
    pub fn books_for(&self, ids: &[u64]) -> Vec<&Book> {
        let by_id: std::collections::HashMap<u64, &Book> =
            self.books.iter().map(|b| (b.id, b)).collect();
        ids.iter().filter_map(|id| by_id.get(id).copied()).collect()
    }

    /// The place of book `id`. Books are appended with rising ids, so a binary search finds it; a list that
    /// is not in id order (or an id that is not there) falls back to the scan, so the answer is always right.
    fn position_of(&self, id: u64) -> Option<usize> {
        let at = self.books.partition_point(|b| b.id < id);
        if self.books.get(at).is_some_and(|b| b.id == id) {
            return Some(at);
        }
        self.books.iter().position(|b| b.id == id)
    }

    pub fn get(&self, id: u64) -> Option<&Book> {
        self.position_of(id).map(|i| &self.books[i])
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Book> {
        self.position_of(id).map(|i| &mut self.books[i])
    }

    pub fn book_path(&self, b: &Book) -> PathBuf {
        self.root.join(&b.file)
    }

    /// The path of the format of `b` that suits `target` best.
    pub fn path_for(&self, b: &Book, target: crate::formats::Target) -> PathBuf {
        self.root.join(b.file_for(target))
    }

    pub fn cover_path(&self, b: &Book) -> Option<PathBuf> {
        b.cover.as_ref().map(|c| self.root.join(c))
    }

    pub fn find_duplicate(&self, meta: &Metadata) -> Option<u64> {
        self.books
            .iter()
            .find(|b| {
                b.meta.title.eq_ignore_ascii_case(&meta.title)
                    && b.meta.authors.first().map(|s| s.to_lowercase())
                        == meta.authors.first().map(|s| s.to_lowercase())
            })
            .map(|b| b.id)
    }

    #[allow(dead_code)]
    pub fn add_path(&mut self, path: &Path, allow_duplicates: bool) -> Result<AddOutcome> {
        let p = prepare_import(path)?;
        let out = self.commit_import(p, allow_duplicates)?;
        self.save()?;
        Ok(out)
    }

    pub fn set_progress(&mut self, id: u64, pos: ReadPos, progress: f64) {
        if let Some(b) = self.get_mut(id) {
            b.position = Some(pos);
            b.progress = progress.clamp(0.0, 1.0);
            b.last_read = Some(Utc::now());
        }
    }

    pub fn mark_read(&mut self, id: u64, read: bool) {
        if let Some(b) = self.get_mut(id) {
            if read {
                b.progress = 1.0;
                b.last_read = Some(Utc::now());
            } else {
                b.progress = 0.0;
                b.position = None;
            }
        }
    }

    pub fn export_copy(&self, id: u64, dest: &Path) -> Result<()> {
        let b = self.get(id).context("no such book")?;
        std::fs::copy(self.book_path(b), dest)?;
        Ok(())
    }

    // -- querying ---------------------------------------------------------

    pub fn search(&self, query: &str, key: SortKey, ascending: bool) -> Vec<u64> {
        let q = Query::parse(query);
        // lower-cased sort keys are computed once per book, not once per comparison
        struct Keyed<'a> {
            b: &'a Book,
            s: String,
            tie: String,
        }
        let mut v: Vec<Keyed> = self
            .books
            .iter()
            .filter(|b| q.matches(b))
            .map(|b| Keyed {
                b,
                s: match key {
                    SortKey::Title => b.meta.title_sort.to_lowercase(),
                    SortKey::Author => b.meta.author_sort.to_lowercase(),
                    SortKey::Series => b.meta.series.to_lowercase(),
                    _ => String::new(),
                },
                tie: b.meta.title_sort.to_lowercase(),
            })
            .collect();
        v.sort_by(|x, y| {
            use std::cmp::Ordering::*;
            let (a, b) = (x.b, y.b);
            let o = match key {
                SortKey::Title | SortKey::Author => x.s.cmp(&y.s),
                SortKey::DateAdded => a.added.cmp(&b.added),
                SortKey::LastRead => a.last_read.cmp(&b.last_read),
                SortKey::Published => a.meta.published.cmp(&b.meta.published),
                SortKey::Rating => a.meta.rating.cmp(&b.meta.rating),
                SortKey::Series => x.s.cmp(&y.s).then(
                    a.meta
                        .series_index
                        .partial_cmp(&b.meta.series_index)
                        .unwrap_or(Equal),
                ),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Progress => a.progress.partial_cmp(&b.progress).unwrap_or(Equal),
            };
            o.then_with(|| x.tie.cmp(&y.tie))
        });
        let mut ids: Vec<u64> = v.iter().map(|k| k.b.id).collect();
        if !ascending {
            ids.reverse();
        }
        ids
    }

    pub fn categories(&self, cat: Category) -> Vec<(String, usize)> {
        let mut map: BTreeMap<String, usize> = BTreeMap::new();
        for b in &self.books {
            match cat {
                Category::Authors => b
                    .meta
                    .authors
                    .iter()
                    .for_each(|a| *map.entry(a.clone()).or_default() += 1),
                Category::Series => {
                    if !b.meta.series.is_empty() {
                        *map.entry(b.meta.series.clone()).or_default() += 1
                    }
                }
                Category::Tags => b
                    .meta
                    .tags
                    .iter()
                    .for_each(|t| *map.entry(t.clone()).or_default() += 1),
                Category::Publishers => {
                    if !b.meta.publisher.is_empty() {
                        *map.entry(b.meta.publisher.clone()).or_default() += 1
                    }
                }
                Category::Languages => {
                    if !b.meta.language.is_empty() {
                        *map.entry(b.meta.language.clone()).or_default() += 1
                    }
                }
                Category::Ratings => {
                    let stars = (b.meta.rating as usize).div_ceil(2);
                    if stars > 0 {
                        *map.entry("★".repeat(stars) + &"☆".repeat(5 - stars))
                            .or_default() += 1
                    }
                }
                Category::Status => *map.entry(b.status().to_string()).or_default() += 1,
            }
        }
        map.into_iter().collect()
    }

    /// Applies a closure to the metadata of several books and writes them to disk.
    pub fn bulk_edit(&mut self, ids: &[u64], f: impl Fn(&mut Metadata)) -> Result<usize> {
        let mut n = 0;
        for id in ids {
            let Some(b) = self.get(*id) else { continue };
            let mut m = b.meta.clone();
            f(&mut m);
            if m != b.meta {
                self.update_metadata(*id, m)?;
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn open_book(&self, id: u64) -> Result<OpenBook> {
        let b = self.get(id).context("no such book")?;
        OpenBook::open(&self.path_for(b, crate::formats::Target::Reader))
    }
}

#[cfg(test)]
mod highlight_style_tests {
    use super::*;

    #[test]
    fn old_catalogues_without_a_style_still_load() {
        let json = r#"{"id":"a","spine":1,"start":3,"end":9,"color":"yellow","note":"","text":"x",
            "created":"2026-01-01T00:00:00Z"}"#;
        let h: Highlight = serde_json::from_str(json).unwrap();
        assert_eq!(h.style, "");
    }

    #[test]
    fn the_style_round_trips() {
        let h = Highlight {
            id: "a".into(),
            spine: 0,
            start: 0,
            end: 4,
            color: "green".into(),
            style: "squiggly".into(),
            note: String::new(),
            text: "text".into(),
            created: Utc::now(),
        };
        let back: Highlight = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(back, h);
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;

    fn book(id: u64, title: &str) -> Book {
        Book {
            id,
            file: format!("{id}.epub"),
            cover: None,
            meta: Metadata {
                title: title.into(),
                ..Default::default()
            },
            added: Utc::now(),
            modified: Utc::now(),
            last_read: None,
            size: 1,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: Vec::new(),
            resources: Vec::new(),
            drm: String::new(),
            custom: Default::default(),
            marked: false,
        }
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-backend-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_new_library_is_empty_and_starts_at_id_one() {
        let d = tmp();
        let lib = Library::in_memory(&d).unwrap();
        assert!(lib.books.is_empty());
        assert_eq!(lib.next_id, 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn saved_books_survive_a_reopen_with_their_edits() {
        let d = tmp();
        let be = MemoryBackend::new();
        let mut lib = Library::with_backend(&d, Box::new(be.clone())).unwrap();
        lib.books.push(book(1, "One"));
        lib.books.push(book(2, "Two"));
        lib.save().unwrap();
        lib.get_mut(2).unwrap().progress = 0.4;
        lib.books.remove(0);
        lib.save().unwrap();
        let again = Library::with_backend(&d, Box::new(be)).unwrap();
        assert_eq!(again.books.len(), 1);
        assert_eq!(again.get(2).unwrap().progress, 0.4);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_id_counter_is_kept_and_never_reuses_an_id() {
        let d = tmp();
        let be = MemoryBackend::new();
        let mut lib = Library::with_backend(&d, Box::new(be.clone())).unwrap();
        lib.books.push(book(5, "Five"));
        lib.next_id = 6;
        lib.save().unwrap();
        lib.books.clear();
        lib.save().unwrap();
        // the library is empty again, but id 5 is not handed out again
        let again = Library::with_backend(&d, Box::new(be)).unwrap();
        assert!(again.books.is_empty());
        assert_eq!(again.next_id, 6);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_counter_is_repaired_when_it_lags_behind_the_books() {
        let d = tmp();
        let mut be = MemoryBackend::new();
        be.commit(1, &[&book(7, "Seven")], &[]).unwrap();
        let lib = Library::with_backend(&d, Box::new(be)).unwrap();
        assert_eq!(lib.next_id, 8);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_unchanged_library_commits_nothing() {
        let d = tmp();
        let be = MemoryBackend::new();
        let mut lib = Library::with_backend(&d, Box::new(be.clone())).unwrap();
        lib.books.push(book(1, "One"));
        lib.save().unwrap();
        // tamper with the backend behind the library's back: an unchanged library must not rewrite it
        be.clone().commit(9, &[], &[1]).unwrap();
        lib.save().unwrap();
        assert_eq!(be.books(), 0, "no commit happened");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_failing_backend_keeps_the_changes_pending() {
        struct Flaky(MemoryBackend, std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl CatalogBackend for Flaky {
            fn load(&mut self) -> Result<(u64, Vec<Book>)> {
                self.0.load()
            }
            fn commit(&mut self, n: u64, up: &[&Book], rm: &[u64]) -> Result<()> {
                if self.1.load(std::sync::atomic::Ordering::SeqCst) {
                    anyhow::bail!("disk full");
                }
                self.0.commit(n, up, rm)
            }
        }
        let d = tmp();
        let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let inner = MemoryBackend::new();
        let mut lib =
            Library::with_backend(&d, Box::new(Flaky(inner.clone(), fail.clone()))).unwrap();
        lib.books.push(book(1, "One"));
        assert!(lib.save().is_err());
        assert_eq!(inner.books(), 0);
        // the failure did not mark the book as saved: the next save writes it
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        lib.save().unwrap();
        assert_eq!(inner.books(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn memory_backend_commit_replaces_by_id_and_stays_sorted() {
        let mut be = MemoryBackend::new();
        be.commit(4, &[&book(3, "C"), &book(1, "A")], &[]).unwrap();
        be.commit(4, &[&book(1, "A2")], &[3]).unwrap();
        let (n, books) = be.load().unwrap();
        assert_eq!(n, 4);
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].meta.title, "A2");
    }
}
