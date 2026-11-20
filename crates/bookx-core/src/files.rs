//! The file-handling half of [`Library`]: import into the chosen layout, transactional moves,
//! the layout change with a dry run, the trash and undo.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;

use crate::archive::Archive;
use crate::book::OpenBook;
use crate::fsops::{self, FileMove};
use crate::history::{self, BookMove, CoverUndo, Op, OpKind, TrashEntry};
use crate::layout::{self, Layout, TemplateFn};
use crate::library::{AddOutcome, Book, Library, Prepared};
use crate::opf::Metadata;
use crate::util::*;

/// A planned move of one book, for the preview and for applying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovePlan {
    pub id: u64,
    pub title: String,
    pub old_file: String,
    pub new_file: String,
    pub old_cover: Option<String>,
    pub new_cover: Option<String>,
    /// Relative paths, applied together.
    pub files: Vec<(String, String)>,
}

impl MovePlan {
    fn is_noop(&self) -> bool {
        self.files.is_empty() && self.old_file == self.new_file
    }
}

fn is_absolute(file: &str) -> bool {
    Path::new(file).is_absolute()
}

/// Writes a file next to its final name first, so a crash never leaves half a book.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("bookx-tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Largest cover kept for undo.
pub const COVER_BACKUP_MAX: usize = 8 * 1024 * 1024;
/// All kept covers together; the oldest go first.
pub const COVER_BACKUP_TOTAL: u64 = 64 * 1024 * 1024;

/// Keeps a cover that is about to be replaced under `.bookx/cover-backups`, within the size caps.
pub fn backup_cover(root: &Path, id: u64, bytes: &[u8], ext: &str) -> CoverUndo {
    backup_cover_capped(root, id, bytes, ext, COVER_BACKUP_MAX, COVER_BACKUP_TOTAL)
}

pub fn backup_cover_capped(
    root: &Path,
    id: u64,
    bytes: &[u8],
    ext: &str,
    max_one: usize,
    max_total: u64,
) -> CoverUndo {
    if bytes.len() > max_one {
        return CoverUndo::TooBig;
    }
    let dir = root.join(history::STATE_DIR).join("cover-backups");
    let name = format!(
        "{}-{id}.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        if ext.is_empty() { "jpg" } else { ext }
    );
    if write_atomic(&dir.join(&name), bytes).is_err() {
        return CoverUndo::TooBig;
    }
    let mut files: Vec<(String, u64)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            Some((
                e.file_name().to_string_lossy().to_string(),
                e.metadata().ok()?.len(),
            ))
        })
        .collect();
    files.sort(); // names start with the time: oldest first
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    for (f, len) in files {
        if total <= max_total || f == name {
            break;
        }
        if std::fs::remove_file(dir.join(&f)).is_ok() {
            total -= len;
        }
    }
    CoverUndo::Saved(format!("{}/cover-backups/{name}", history::STATE_DIR))
}

/// All files below `dir`, as paths relative to `base`.
fn files_below(base: &Path, dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

impl Library {
    // -- settings -----------------------------------------------------------------------

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Sets the layout used for new books and remembers it in the library folder. Existing books
    /// move only with [`Library::change_layout`].
    pub fn set_layout(&mut self, layout: Layout) -> Result<()> {
        layout::save_layout(&self.root, &layout).context("cannot remember the layout")?;
        self.layout = layout;
        Ok(())
    }

    /// The function that renders [`Layout::Template`] paths for this library, overriding the one
    /// registered for the process (the template language lives in `bookx-sources`, which this
    /// crate cannot depend on).
    pub fn set_template_renderer(&mut self, f: TemplateFn) {
        self.template_fn = Some(f);
    }

    pub(crate) fn renderer(&self) -> Option<TemplateFn> {
        self.template_fn.or_else(layout::global_renderer)
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// A read-only library never writes the catalogue or touches files.
    pub fn set_read_only(&mut self, on: bool) {
        self.read_only = on;
    }

    pub(crate) fn writable(&self) -> Result<()> {
        if self.read_only {
            bail!("this library is open read-only; close the other Bookx that uses it first");
        }
        Ok(())
    }

    // -- operation log ---------------------------------------------------------------------

    pub(crate) fn log(&mut self, label: String, kind: OpKind) {
        if self.replaying {
            return;
        }
        history::push_op(&mut self.ops, label, kind, Utc::now());
        // the log is a convenience: a full disk must not fail the operation that was logged
        let _ = history::save_ops(&self.root, &self.ops);
    }

    /// The operations, newest last.
    pub fn history(&self) -> &[Op] {
        &self.ops
    }

    /// Takes back the newest operation that is not undone yet. Returns its label.
    pub fn undo_last(&mut self) -> Result<Option<String>> {
        self.writable()?;
        let Some(op) = history::last_undoable(&self.ops).cloned() else {
            return Ok(None);
        };
        self.replaying = true;
        let r = self.undo_kind(&op.kind);
        self.replaying = false;
        r.with_context(|| format!("cannot undo \"{}\"", op.label))?;
        if let Some(o) = self.ops.iter_mut().find(|o| o.seq == op.seq) {
            o.undone = true;
        }
        let _ = history::save_ops(&self.root, &self.ops);
        Ok(Some(op.label))
    }

    fn undo_kind(&mut self, kind: &OpKind) -> Result<()> {
        match kind {
            OpKind::Delete { trash } => {
                for name in trash {
                    self.restore_from_trash(name)?;
                }
                Ok(())
            }
            OpKind::Import { ids } => {
                let present: Vec<u64> = ids
                    .iter()
                    .copied()
                    .filter(|i| self.get(*i).is_some())
                    .collect();
                self.remove(&present, true)
            }
            OpKind::Edit { id, before, cover } => {
                if let Some(c) = cover {
                    self.restore_cover(*id, c)?;
                }
                self.update_metadata(*id, (**before).clone())
            }
            OpKind::EditMany { edits } => {
                for (id, before) in edits {
                    if self.get(*id).is_some() {
                        self.update_metadata(*id, (**before).clone())?;
                    }
                }
                Ok(())
            }
            OpKind::Move { moves } => self.undo_moves(moves),
            OpKind::RepairHighlights { before } => self.undo_repair_highlights(before),
            OpKind::MergeFormats {
                primary,
                before_file,
                before_formats,
                merged,
            } => self.undo_merge(*primary, before_file, before_formats, merged),
            OpKind::SetResources { id, before } => {
                if let Some(b) = self.get_mut(*id) {
                    b.resources = before.clone();
                }
                self.save()
            }
            OpKind::AddFormat { id, file } => self.undo_add_format(*id, file),
            OpKind::RemoveFormat {
                id,
                format,
                trashed,
            } => self.undo_remove_format(*id, format, trashed),
        }
    }

    // -- importing -------------------------------------------------------------------------

    fn taken_files(&self) -> HashSet<String> {
        self.books.iter().map(|b| b.file.clone()).collect()
    }

    fn place_of(&self, meta: &Metadata, id: u64, taken: &HashSet<String>) -> Option<layout::Place> {
        layout::plan(&self.layout, meta, id, "epub", self.renderer(), &|f| {
            taken.contains(f) || self.root.join(f).exists()
        })
    }

    /// Registers an imported book. Callers must call [`Library::save`] afterwards.
    pub fn commit_import(&mut self, p: Prepared, allow_duplicates: bool) -> Result<AddOutcome> {
        self.writable()?;
        if !allow_duplicates {
            if let Some(id) = self.find_duplicate(&p.meta) {
                return Ok(AddOutcome::Duplicate(id));
            }
        }
        let source_pdf = p
            .source_file
            .clone()
            .filter(|s| s.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")));
        let id = self.next_id;
        let taken = self.taken_files();
        let mut written: Vec<PathBuf> = Vec::new();
        let result = (|| -> Result<(String, Option<String>)> {
            let (file, book_dir): (String, Option<String>) = match (&self.layout, &p.origin) {
                (Layout::Linked, Some(origin)) => {
                    let abs = origin.canonicalize().unwrap_or_else(|_| origin.clone());
                    (abs.to_string_lossy().to_string(), None)
                }
                _ => {
                    // Linked cannot keep a converted book where it was: it goes into a Tree
                    let place = match self.place_of(&p.meta, id, &taken) {
                        Some(pl) => pl,
                        None => layout::plan(&Layout::Tree, &p.meta, id, "epub", None, &|f| {
                            taken.contains(f)
                        })
                        .context("no place for the book")?,
                    };
                    let path = self.root.join(&place.file);
                    write_atomic(&path, &p.bytes)
                        .with_context(|| format!("cannot write {}", path.display()))?;
                    written.push(path);
                    (place.file, place.book_dir)
                }
            };
            let cover = match &p.cover {
                Some((data, ext)) => {
                    let rel = layout::cover_rel(book_dir.as_deref(), id, ext);
                    let path = self.root.join(&rel);
                    write_atomic(&path, data)
                        .with_context(|| format!("cannot write {}", path.display()))?;
                    written.push(path);
                    Some(rel)
                }
                None => None,
            };
            Ok((file, cover))
        })();
        let (file, cover) = match result {
            Ok(v) => v,
            Err(e) => {
                for w in &written {
                    let _ = std::fs::remove_file(w);
                    if let Some(d) = w.parent() {
                        fsops::prune_empty_dirs(d, &self.root);
                    }
                }
                return Err(e);
            }
        };
        let now = Utc::now();
        self.next_id += 1;
        let label = format!("Imported \"{}\"", p.meta.title);
        self.books.push(Book {
            id,
            file,
            cover,
            meta: p.meta,
            added: now,
            modified: now,
            last_read: None,
            size: p.bytes.len() as u64,
            progress: 0.0,
            position: None,
            bookmarks: Vec::new(),
            highlights: Vec::new(),
            formats: Vec::new(),
            resources: Vec::new(),
            drm: p.drm,
        });
        self.log(label, OpKind::Import { ids: vec![id] });
        // a PDF is shown from the file itself (page view); the EPUB made from it is the reflowed text
        if let Some(src) = source_pdf {
            let _ = self.add_format(id, &src);
        }
        Ok(AddOutcome::Added(id))
    }

    /// Copies a linked book (and its cover) into the library folder according to the current
    /// layout, so the library no longer depends on the folder it came from (for example after a
    /// Calibre import). The original files stay where they were.
    pub fn copy_into_library(&mut self, id: u64) -> Result<()> {
        self.writable()?;
        let b = self.get(id).context("no such book")?.clone();
        if !is_absolute(&b.file) {
            return Ok(());
        }
        if !self.layout.manages_files() {
            return Ok(());
        }
        let taken = self.taken_files();
        let place = layout::plan(&self.layout, &b.meta, id, "epub", self.renderer(), &|f| {
            taken.contains(f) || self.root.join(f).exists()
        })
        .context("no place for the book")?;
        let dest = self.root.join(&place.file);
        let mut written = vec![dest.clone()];
        let r = (|| -> Result<Option<String>> {
            let data = std::fs::read(&b.file).with_context(|| format!("cannot read {}", b.file))?;
            write_atomic(&dest, &data)?;
            match &b.cover {
                Some(c) => {
                    let rel = layout::cover_rel(place.book_dir.as_deref(), id, &layout::ext_of(c));
                    let target = self.root.join(&rel);
                    match std::fs::read(c) {
                        Ok(d) => {
                            written.push(target.clone());
                            write_atomic(&target, &d)?;
                            Ok(Some(rel))
                        }
                        Err(_) => Ok(None), // a missing cover is not worth failing the copy
                    }
                }
                None => Ok(None),
            }
        })();
        match r {
            Ok(cover) => {
                if let Some(bk) = self.get_mut(id) {
                    bk.file = place.file;
                    bk.cover = cover;
                }
                Ok(())
            }
            Err(e) => {
                for w in &written {
                    let _ = std::fs::remove_file(w);
                    if let Some(d) = w.parent() {
                        fsops::prune_empty_dirs(d, &self.root);
                    }
                }
                Err(e)
            }
        }
    }

    // -- delete, trash --------------------------------------------------------------------

    /// The folder that belongs to this book alone (`Author/Title (id)`), if it has one.
    pub fn book_dir_of(&self, b: &Book) -> Option<String> {
        if is_absolute(&b.file) {
            return None;
        }
        let dir = Path::new(&b.file).parent()?.to_string_lossy().to_string();
        let leaf = Path::new(&dir).file_name()?.to_string_lossy().to_string();
        (!dir.is_empty() && leaf.ends_with(&format!(" ({})", b.id))).then_some(dir)
    }

    /// Files of a book that live in the library folder, relative to it.
    fn owned_files(&self, b: &Book) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(dir) = self.book_dir_of(b) {
            out.extend(files_below(&self.root, &self.root.join(&dir)));
        } else if !is_absolute(&b.file) && self.root.join(&b.file).exists() {
            out.push(b.file.clone());
        }
        if let Some(c) = &b.cover {
            if !is_absolute(c) && !out.contains(c) && self.root.join(c).exists() {
                out.push(c.clone());
            }
        }
        for f in &b.formats {
            if !is_absolute(&f.file) && !out.contains(&f.file) && self.root.join(&f.file).exists() {
                out.push(f.file.clone());
            }
        }
        out
    }

    /// Removes books from the catalogue. With `delete_files` their files go to the trash (never
    /// erased; see [`Library::empty_trash`]); files of linked books are never touched. All files
    /// of the call move together or not at all.
    pub fn remove(&mut self, ids: &[u64], delete_files: bool) -> Result<()> {
        self.writable()?;
        let trash_root = history::trash_dir(&self.root);
        let stamp = Utc::now().timestamp_millis();
        let mut entries: Vec<(TrashEntry, Vec<FileMove>)> = Vec::new();
        for id in ids {
            let Some(b) = self.get(*id) else { continue };
            let name = format!("{stamp}-{id}");
            let files = if delete_files {
                self.owned_files(b)
            } else {
                Vec::new()
            };
            let moves = files
                .iter()
                .map(|f| {
                    FileMove::new(
                        self.root.join(f),
                        trash_root.join(&name).join("files").join(f),
                    )
                })
                .collect();
            entries.push((
                TrashEntry {
                    name,
                    at: Utc::now(),
                    book: b.clone(),
                    files,
                },
                moves,
            ));
        }
        if entries.is_empty() {
            return self.save();
        }
        // entry records first (cheap to undo), then the files in one transaction
        let mut made: Vec<PathBuf> = Vec::new();
        let r = (|| -> Result<()> {
            for (e, _) in &entries {
                let dir = trash_root.join(&e.name);
                std::fs::create_dir_all(&dir)?;
                made.push(dir.clone());
                std::fs::write(dir.join("entry.json"), serde_json::to_vec_pretty(e)?)?;
            }
            let all: Vec<FileMove> = entries.iter().flat_map(|(_, m)| m.clone()).collect();
            fsops::apply(&all)
        })();
        if let Err(e) = r {
            for d in made {
                let _ = std::fs::remove_dir_all(d);
            }
            return Err(e.context("nothing was deleted"));
        }
        let mut names = Vec::new();
        let mut title = String::new();
        for (e, _) in &entries {
            title = e.book.meta.title.clone();
            names.push(e.name.clone());
            self.books.retain(|b| b.id != e.book.id);
            for f in &e.files {
                if let Some(d) = Path::new(f).parent() {
                    fsops::prune_empty_dirs(&self.root.join(d), &self.root);
                }
            }
        }
        let label = if names.len() == 1 {
            format!("Deleted \"{title}\"")
        } else {
            format!("Deleted {} books", names.len())
        };
        self.log(label, OpKind::Delete { trash: names });
        self.save()
    }

    /// The books in the trash, newest first.
    pub fn trash(&self) -> Vec<TrashEntry> {
        let mut out: Vec<TrashEntry> = std::fs::read_dir(history::trash_dir(&self.root))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read(e.path().join("entry.json")).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        out.sort_by(|a, b| b.at.cmp(&a.at));
        out
    }

    /// Puts a deleted book back: its files return to where they were and it is listed again.
    pub fn restore_from_trash(&mut self, name: &str) -> Result<u64> {
        self.writable()?;
        let dir = history::trash_dir(&self.root).join(name);
        let entry: TrashEntry = serde_json::from_slice(
            &std::fs::read(dir.join("entry.json")).context("that book is not in the trash")?,
        )
        .context("the trash record is damaged")?;
        let id = entry.book.id;
        if self.get(id).is_some() {
            bail!("a book with id {id} exists already");
        }
        let moves: Vec<FileMove> = entry
            .files
            .iter()
            .map(|f| FileMove::new(dir.join("files").join(f), self.root.join(f)))
            .collect();
        fsops::apply(&moves).context("cannot put the files back")?;
        self.next_id = self.next_id.max(id + 1);
        self.books.push(entry.book);
        std::fs::remove_dir_all(&dir).with_context(|| format!("cannot clean {}", dir.display()))?;
        self.save()?;
        Ok(id)
    }

    /// Erases everything in the trash for good. Returns the number of books.
    pub fn empty_trash(&mut self) -> Result<usize> {
        self.writable()?;
        let n = self.trash().len();
        let dir = history::trash_dir(&self.root);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .with_context(|| format!("cannot empty {}", dir.display()))?;
        }
        Ok(n)
    }

    // -- metadata edits ----------------------------------------------------------------------

    /// Writes `meta` into the book's EPUB, refreshes the catalogue and moves the files if needed.
    pub fn update_metadata(&mut self, id: u64, meta: Metadata) -> Result<()> {
        self.writable()?;
        let (path, before) = {
            let b = self.get(id).context("no such book")?;
            (self.book_path(b), b.meta.clone())
        };
        let mut book = OpenBook::open(&path)?;
        book.set_metadata(meta)?;
        book.archive.save(&path)?;
        let cover = book
            .package
            .cover_path()
            .and_then(|p| book.archive.get(&p).map(|b| (b.to_vec(), extension(&p))));
        let new_meta = book.metadata().clone();
        let label = format!("Edited \"{}\"", new_meta.title);
        let cover_undo = self.refresh_after_edit(id, new_meta, cover)?;
        self.log(
            label,
            OpKind::Edit {
                id,
                before: Box::new(before),
                cover: cover_undo,
            },
        );
        Ok(())
    }

    /// Edits several books and logs ONE operation, so a single undo takes all of them back
    /// (renaming an author everywhere). `edits` are `(id, new metadata)`; books that do not
    /// change are skipped. Returns the number of books changed.
    pub fn update_metadata_many(
        &mut self,
        label: String,
        edits: Vec<(u64, Metadata)>,
    ) -> Result<usize> {
        self.writable()?;
        let replaying = self.replaying;
        self.replaying = true;
        let mut before: Vec<(u64, Box<Metadata>)> = Vec::new();
        let mut result = Ok(());
        for (id, meta) in edits {
            let Some(b) = self.get(id) else { continue };
            if b.meta == meta {
                continue;
            }
            let old = b.meta.clone();
            match self.update_metadata(id, meta) {
                Ok(()) => before.push((id, Box::new(old))),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.replaying = replaying;
        let n = before.len();
        if n > 0 {
            self.log(label, OpKind::EditMany { edits: before });
        }
        result.map(|_| n)
    }

    /// Puts a replaced cover back into the book file and the library (undo of an edit).
    fn restore_cover(&mut self, id: u64, c: &CoverUndo) -> Result<()> {
        let path = {
            let b = self.get(id).context("no such book")?;
            self.book_path(b)
        };
        match c {
            CoverUndo::Absent => {
                let mut book = OpenBook::open(&path)?;
                book.remove_cover()?;
                book.archive.save(&path)?;
            }
            CoverUndo::Saved(rel) => {
                let data = std::fs::read(self.root.join(rel))
                    .context("the saved cover is gone; only the metadata can be restored")?;
                let mut book = OpenBook::open(&path)?;
                book.set_cover(data, &layout::ext_of(rel))?;
                book.archive.save(&path)?;
            }
            CoverUndo::TooBig => {}
        }
        Ok(())
    }

    /// Replaces the book's file with an edited archive (used by the book editor).
    pub fn replace_archive(&mut self, id: u64, archive: &Archive) -> Result<()> {
        self.writable()?;
        let path = {
            let b = self.get(id).context("no such book")?;
            self.book_path(b)
        };
        archive.save(&path)?;
        let book = OpenBook::from_archive(archive.clone())?;
        let cover = book
            .package
            .cover_path()
            .and_then(|p| book.archive.get(&p).map(|b| (b.to_vec(), extension(&p))));
        self.refresh_after_edit(id, book.metadata().clone(), cover)
            .map(|_| ())
    }

    fn refresh_after_edit(
        &mut self,
        id: u64,
        meta: Metadata,
        cover: Option<(Vec<u8>, String)>,
    ) -> Result<Option<CoverUndo>> {
        let root = self.root.clone();
        let book_dir = self.get(id).and_then(|b| self.book_dir_of(b));
        let mut cover_undo = None;
        {
            let b = self.get_mut(id).context("no such book")?;
            b.meta = meta;
            b.modified = Utc::now();
            let path = root.join(&b.file);
            b.size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(b.size);
            match cover {
                Some((data, ext)) => {
                    let ext = ext.replace("jpeg", "jpg");
                    let rel = layout::cover_rel(book_dir.as_deref(), id, &ext);
                    cover_undo = match &b.cover {
                        Some(old) => match std::fs::read(root.join(old)) {
                            Ok(bytes) if bytes != data => {
                                Some(backup_cover(&root, id, &bytes, &layout::ext_of(old)))
                            }
                            _ => None,
                        },
                        None => Some(CoverUndo::Absent),
                    };
                    if let Some(old) = &b.cover {
                        if *old != rel && !is_absolute(old) {
                            let _ = std::fs::remove_file(root.join(old));
                        }
                    }
                    write_atomic(&root.join(&rel), &data)?;
                    b.cover = Some(rel);
                }
                None => {
                    if let Some(old) = b.cover.take() {
                        if let Ok(bytes) = std::fs::read(root.join(&old)) {
                            cover_undo =
                                Some(backup_cover(&root, id, &bytes, &layout::ext_of(&old)));
                        }
                        if !is_absolute(&old) {
                            let _ = std::fs::remove_file(root.join(old));
                        }
                    }
                }
            }
        }
        self.reorganize(id)?;
        self.save()?;
        Ok(cover_undo)
    }

    // -- moving files ----------------------------------------------------------------------

    /// Plans where one book should be for the layout `layout`; `None` when it is already there
    /// (or is linked). `taken` holds the files other books use or will use.
    fn plan_book(&self, b: &Book, layout: &Layout, taken: &HashSet<String>) -> Option<MovePlan> {
        if is_absolute(&b.file) || !layout.manages_files() {
            return None;
        }
        let ext = layout::ext_of(&b.file);
        let place = layout::plan(layout, &b.meta, b.id, &ext, self.renderer(), &|f| {
            taken.contains(f) || (f != b.file && self.root.join(f).exists())
        })?;
        let old_dir = self.book_dir_of(b);
        let mut files: Vec<(String, String)> = Vec::new();
        let new_cover = b
            .cover
            .as_ref()
            .map(|c| layout::cover_rel(place.book_dir.as_deref(), b.id, &layout::ext_of(c)));
        match (&old_dir, &place.book_dir) {
            // folder to folder: everything in it moves along (sidecar files, extra covers)
            (Some(od), Some(nd)) => {
                for f in files_below(&self.root, &self.root.join(od)) {
                    let rest = f.strip_prefix(&format!("{od}/")).unwrap_or(&f).to_string();
                    let target = if f == b.file {
                        place.file.clone()
                    } else if Some(&f) == b.cover.as_ref() {
                        new_cover.clone().unwrap_or_else(|| format!("{nd}/{rest}"))
                    } else {
                        format!("{nd}/{rest}")
                    };
                    if target != f {
                        files.push((f, target));
                    }
                }
            }
            _ => {
                if place.file != b.file {
                    files.push((b.file.clone(), place.file.clone()));
                }
                if let (Some(old), Some(new)) = (&b.cover, &new_cover) {
                    if old != new {
                        files.push((old.clone(), new.clone()));
                    }
                }
            }
        }
        let plan = MovePlan {
            id: b.id,
            title: b.meta.title.clone(),
            old_file: b.file.clone(),
            new_file: place.file,
            old_cover: b.cover.clone(),
            new_cover,
            files,
        };
        (!plan.is_noop()).then_some(plan)
    }

    /// Dry run: what would move if the library used `layout`. Nothing is touched.
    pub fn plan_layout(&self, layout: &Layout) -> Vec<MovePlan> {
        let mut taken: HashSet<String> = self.books.iter().map(|b| b.file.clone()).collect();
        let mut order: Vec<&Book> = self.books.iter().collect();
        order.sort_by_key(|b| b.id);
        let mut plans = Vec::new();
        for b in order {
            taken.remove(&b.file);
            match self.plan_book(b, layout, &taken) {
                Some(p) => {
                    taken.insert(p.new_file.clone());
                    plans.push(p);
                }
                None => {
                    taken.insert(b.file.clone());
                }
            }
        }
        plans
    }

    /// Applies planned moves as one transaction; on any failure every file is back where it was
    /// and the catalogue is unchanged.
    pub fn apply_plan(&mut self, plans: &[MovePlan]) -> Result<usize> {
        self.apply_plan_with(plans, &mut |a, b| fsops::move_file(a, b))
    }

    /// Like [`Library::apply_plan`] with the mover injected (tests simulate a failing disk).
    pub fn apply_plan_with(
        &mut self,
        plans: &[MovePlan],
        mover: &mut dyn FnMut(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<usize> {
        self.writable()?;
        let moves: Vec<FileMove> = plans
            .iter()
            .flat_map(|p| &p.files)
            .map(|(a, b)| FileMove::new(self.root.join(a), self.root.join(b)))
            .collect();
        fsops::apply_with(&moves, mover)?;
        let mut done = Vec::new();
        for p in plans {
            if let Some(b) = self.get_mut(p.id) {
                b.file = p.new_file.clone();
                b.cover = p.new_cover.clone();
                for f in &mut b.formats {
                    if let Some((_, to)) = p.files.iter().find(|(from, _)| *from == f.file) {
                        f.file = to.clone();
                    }
                }
            }
            for (from, _) in &p.files {
                if let Some(d) = Path::new(from).parent() {
                    fsops::prune_empty_dirs(&self.root.join(d), &self.root);
                }
            }
            done.push(BookMove {
                id: p.id,
                old_file: p.old_file.clone(),
                new_file: p.new_file.clone(),
                old_cover: p.old_cover.clone(),
                new_cover: p.new_cover.clone(),
                files: p.files.clone(),
            });
        }
        let n = done.len();
        if n > 0 {
            self.log(format!("Moved {n} books"), OpKind::Move { moves: done });
        }
        self.save()?;
        Ok(n)
    }

    /// Switches to `layout` and moves every book there, all or nothing. If the move fails the
    /// old layout stays.
    pub fn change_layout(&mut self, layout: Layout) -> Result<usize> {
        self.writable()?;
        let plans = self.plan_layout(&layout);
        let n = self.apply_plan(&plans)?;
        self.set_layout(layout)?;
        Ok(n)
    }

    /// Moves one book into the place its metadata asks for (a no-op for linked books).
    pub(crate) fn reorganize(&mut self, id: u64) -> Result<()> {
        let layout = self.layout.clone();
        let taken: HashSet<String> = self
            .books
            .iter()
            .filter(|b| b.id != id)
            .map(|b| b.file.clone())
            .collect();
        let plan = {
            let b = self.get(id).context("no such book")?;
            self.plan_book(b, &layout, &taken)
        };
        let Some(plan) = plan else { return Ok(()) };
        let replaying = self.replaying;
        self.replaying = true; // an edit is logged as the edit, not as a move
        let r = self.apply_plan(std::slice::from_ref(&plan));
        self.replaying = replaying;
        r.map(|_| ())
    }

    fn undo_moves(&mut self, moves: &[BookMove]) -> Result<()> {
        let fs: Vec<FileMove> = moves
            .iter()
            .flat_map(|m| &m.files)
            .map(|(a, b)| FileMove::new(self.root.join(b), self.root.join(a)))
            .collect();
        fsops::apply(&fs)?;
        for m in moves {
            if let Some(b) = self.get_mut(m.id) {
                b.file = m.old_file.clone();
                b.cover = m.old_cover.clone();
                for f in &mut b.formats {
                    if let Some((from, _)) = m.files.iter().find(|(_, to)| *to == f.file) {
                        f.file = from.clone();
                    }
                }
            }
            for (_, to) in &m.files {
                if let Some(d) = Path::new(to).parent() {
                    fsops::prune_empty_dirs(&self.root.join(d), &self.root);
                }
            }
        }
        self.save()
    }
}
