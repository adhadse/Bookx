//! Several format files for one book: merging books into one, adding a format file, removing one.
//!
//! Every step is one operation in the log ([`OpKind`]) and can be undone. Nothing is deleted:
//! merged catalogue records go to the trash, removed format files wait in `.bookx-trash/formats`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::formats::{self, FormatFile, Target};
use crate::fsops::{self, FileMove};
use crate::history::{self, MergedBook, OpKind};
use crate::library::{Book, Library};
use crate::opf::Metadata;

/// One book of a planned merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeItem {
    pub id: u64,
    pub title: String,
    pub formats: Vec<String>,
    pub size: u64,
    /// Why this book cannot be merged; it is skipped.
    pub blocked: Option<String>,
}

/// What *Merge as formats of one book* would do. Nothing is changed by planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePlan {
    pub primary: u64,
    pub title: String,
    pub primary_formats: Vec<String>,
    pub items: Vec<MergeItem>,
    /// What is not carried over, shown in the preview.
    pub notes: Vec<String>,
}

impl MergePlan {
    pub fn mergeable(&self) -> Vec<&MergeItem> {
        self.items.iter().filter(|i| i.blocked.is_none()).collect()
    }

    /// The formats the book will have, primary first.
    pub fn resulting_formats(&self) -> Vec<String> {
        let mut v = self.primary_formats.clone();
        for i in self.mergeable() {
            v.extend(i.formats.iter().cloned());
        }
        v
    }

    /// One line for the preview: `Dune: epub + azw3 + pdf`.
    pub fn summary(&self) -> String {
        format!("{}: {}", self.title, self.resulting_formats().join(" + "))
    }
}

fn is_absolute(f: &str) -> bool {
    Path::new(f).is_absolute()
}

fn unused_name(root: &Path, dir: &str, name: &str, taken: &[String]) -> String {
    let (stem, ext) = match name.split_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    let mut n = 1;
    loop {
        let cand = if n == 1 {
            format!("{dir}/{stem}{ext}")
        } else {
            format!("{dir}/{stem} ({n}){ext}")
        };
        if !root.join(&cand).exists() && !taken.contains(&cand) {
            return cand;
        }
        n += 1;
    }
}

impl Library {
    /// Dry run of a merge of `others` into `primary`.
    pub fn plan_merge(&self, primary: u64, others: &[u64]) -> Result<MergePlan> {
        let p = self.get(primary).context("no such book")?;
        let mut have: Vec<String> = p.all_formats().into_iter().map(|f| f.format).collect();
        let primary_formats = have.clone();
        let mut items = Vec::new();
        let mut seen = vec![primary];
        for id in others {
            let Some(b) = self.get(*id) else { continue };
            let formats: Vec<String> = b.all_formats().into_iter().map(|f| f.format).collect();
            let blocked = if seen.contains(id) {
                Some("already part of this merge".to_string())
            } else {
                formats.iter().find(|f| have.contains(f)).map(|dup| {
                    format!(
                        "both have a {} file: they are editions or copies, not formats",
                        dup.to_uppercase()
                    )
                })
            };
            seen.push(*id);
            if blocked.is_none() {
                have.extend(formats.iter().cloned());
            }
            items.push(MergeItem {
                id: *id,
                title: b.meta.title.clone(),
                formats,
                size: b.size,
                blocked,
            });
        }
        Ok(MergePlan {
            primary,
            title: p.meta.title.clone(),
            primary_formats,
            items,
            notes: vec![
                "The metadata, cover and reading position of the first book are kept.".into(),
                "Highlights, bookmarks and the reading position of the merged copies stay in the Trash record; Undo brings them back.".into(),
            ],
        })
    }

    /// Folds the books `others` into `primary` as further formats of it. Returns how many were merged.
    pub fn merge_formats(&mut self, primary: u64, others: &[u64]) -> Result<usize> {
        self.writable()?;
        let plan = self.plan_merge(primary, others)?;
        let items: Vec<MergeItem> = plan.mergeable().into_iter().cloned().collect();
        if items.is_empty() {
            bail!("nothing to merge: the books have no format the first one lacks");
        }
        let (before_file, before_formats, dir, title) = {
            let p = self.get(primary).context("no such book")?;
            (
                p.file.clone(),
                p.formats.clone(),
                self.book_dir_of(p),
                p.meta.title.clone(),
            )
        };
        // where each format file will live
        let mut taken: Vec<String> = Vec::new();
        let mut moves: Vec<FileMove> = Vec::new();
        let mut targets: Vec<(u64, String, String)> = Vec::new(); // id, old, new
        for it in &items {
            let b = self.get(it.id).context("no such book")?.clone();
            let new_file = match &dir {
                Some(d) if !is_absolute(&b.file) => {
                    let name = Path::new(&b.file)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| b.file.clone());
                    let n = unused_name(&self.root, d, &name, &taken);
                    taken.push(n.clone());
                    n
                }
                _ => b.file.clone(),
            };
            if new_file != b.file {
                moves.push(FileMove::new(
                    self.root.join(&b.file),
                    self.root.join(&new_file),
                ));
            }
            targets.push((it.id, b.file.clone(), new_file));
        }
        fsops::apply(&moves).context("cannot move the format files; nothing was changed")?;

        let replaying = self.replaying;
        self.replaying = true;
        let mut merged: Vec<MergedBook> = Vec::new();
        let mut all: Vec<FormatFile> = self.get(primary).map(Book::all_formats).unwrap_or_default();
        let mut failure = None;
        for (id, old, new) in &targets {
            let b = self.get(*id).cloned();
            let Some(b) = b else { continue };
            let moved = old != new;
            if let Err(e) = self.remove(&[*id], moved) {
                failure = Some(e);
                break;
            }
            let trash = self
                .trash()
                .into_iter()
                .find(|t| t.book.id == *id)
                .map(|t| t.name)
                .unwrap_or_default();
            merged.push(MergedBook {
                trash,
                old_file: old.clone(),
                new_file: new.clone(),
            });
            for mut f in b.all_formats() {
                if f.file == *old {
                    f.file = new.clone();
                }
                all.push(f);
            }
        }
        self.replaying = replaying;
        if let Some(e) = failure {
            // put back what was done so far
            self.replaying = true;
            let _ = self.undo_merge(primary, &before_file, &before_formats, &merged);
            // files not yet handled by `remove` still sit in their new place
            for (id, old, new) in &targets {
                if old != new && self.get(*id).is_some() {
                    let _ = fsops::move_file(&self.root.join(new), &self.root.join(old));
                }
            }
            self.replaying = replaying;
            return Err(e.context("the merge was rolled back"));
        }
        let first = formats::reader_index(&all);
        let main = all.remove(first);
        if let Some(b) = self.get_mut(primary) {
            b.file = main.file;
            b.size = main.size;
            b.formats = all;
            b.modified = Utc::now();
        }
        let n = merged.len();
        self.log(
            format!("Merged {n} books as formats of \"{title}\""),
            OpKind::MergeFormats {
                primary,
                before_file,
                before_formats,
                merged,
            },
        );
        self.save()?;
        Ok(n)
    }

    pub(crate) fn undo_merge(
        &mut self,
        primary: u64,
        before_file: &str,
        before_formats: &[FormatFile],
        merged: &[MergedBook],
    ) -> Result<()> {
        for m in merged {
            if m.new_file != m.old_file && self.root.join(&m.new_file).exists() {
                fsops::apply(&[FileMove::new(
                    self.root.join(&m.new_file),
                    self.root.join(&m.old_file),
                )])?;
            }
            if !m.trash.is_empty() {
                self.restore_from_trash(&m.trash)?;
            }
        }
        let size = std::fs::metadata(self.root.join(before_file))
            .map(|m| m.len())
            .ok();
        if let Some(b) = self.get_mut(primary) {
            b.file = before_file.to_string();
            b.formats = before_formats.to_vec();
            if let Some(s) = size {
                b.size = s;
            }
        }
        self.save()
    }

    /// The book that `meta` (a file about to be imported) is another copy of, by identifier
    /// (ISBN or any shared scheme) first and then by title and first author.
    pub fn find_same_book(&self, meta: &Metadata) -> Option<u64> {
        for b in &self.books {
            for i in &meta.identifiers {
                if !i.value.trim().is_empty()
                    && b.meta.identifiers.iter().any(|j| {
                        j.scheme.eq_ignore_ascii_case(&i.scheme)
                            && j.value
                                .replace('-', "")
                                .eq_ignore_ascii_case(&i.value.replace('-', ""))
                            && !matches!(i.scheme.to_lowercase().as_str(), "uuid" | "calibre")
                    })
                {
                    return Some(b.id);
                }
            }
        }
        self.find_duplicate(meta)
    }

    /// Adds the file `src` to the book as a further format. In a managed library the file is
    /// copied next to the book; a linked library keeps pointing at `src`.
    pub fn add_format(&mut self, id: u64, src: &Path) -> Result<FormatFile> {
        self.writable()?;
        let b = self.get(id).context("no such book")?.clone();
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .context("not a file")?;
        let format = formats::format_of(&name);
        if format.is_empty() {
            bail!("{name} has no file extension");
        }
        if b.all_formats().iter().any(|f| f.format == format) {
            bail!("the book has a {} file already", format.to_uppercase());
        }
        let size = std::fs::metadata(src)
            .with_context(|| format!("cannot read {}", src.display()))?
            .len();
        let file = if is_absolute(&b.file) {
            src.canonicalize()
                .unwrap_or_else(|_| src.to_path_buf())
                .to_string_lossy()
                .to_string()
        } else {
            let dir = Path::new(&b.file)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let stem = Path::new(&b.file)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "book".into());
            let ext = if name.to_lowercase().ends_with(".kepub.epub") {
                "kepub.epub".to_string()
            } else {
                Path::new(&name)
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            let target = unused_name(&self.root, &dir, &format!("{stem}.{ext}"), &[]);
            let target = target.trim_start_matches('/').to_string();
            let dest = self.root.join(&target);
            if let Some(d) = dest.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::copy(src, &dest)
                .with_context(|| format!("cannot copy {} into the library", src.display()))?;
            target
        };
        let f = FormatFile { file, format, size };
        if let Some(bk) = self.get_mut(id) {
            bk.formats.push(f.clone());
        }
        self.log(
            format!("Added {} to \"{}\"", f.format.to_uppercase(), b.meta.title),
            OpKind::AddFormat {
                id,
                file: f.file.clone(),
            },
        );
        self.save()?;
        Ok(f)
    }

    pub(crate) fn undo_add_format(&mut self, id: u64, file: &str) -> Result<()> {
        if let Some(b) = self.get_mut(id) {
            b.formats.retain(|f| f.file != file);
        }
        if !is_absolute(file) {
            let p = self.root.join(file);
            let _ = std::fs::remove_file(&p);
            if let Some(d) = p.parent() {
                fsops::prune_empty_dirs(d, &self.root);
            }
        }
        self.save()
    }

    /// Takes one further format off a book. The file waits in the trash folder (a linked file is
    /// only unlinked). The primary file cannot be removed this way.
    pub fn remove_format(&mut self, id: u64, file: &str) -> Result<()> {
        self.writable()?;
        let b = self.get(id).context("no such book")?.clone();
        let Some(f) = b.formats.iter().find(|f| f.file == file).cloned() else {
            bail!("that is the main file of the book or not a format of it");
        };
        let trashed = if is_absolute(file) {
            String::new()
        } else {
            let rel = format!(
                "{}/formats/{}-{}",
                history::TRASH_DIR,
                Utc::now().timestamp_millis(),
                f.file.replace('/', "_")
            );
            let to = self.root.join(&rel);
            fsops::apply(&[FileMove::new(self.root.join(file), &to)])?;
            if let Some(d) = Path::new(file).parent() {
                fsops::prune_empty_dirs(&self.root.join(d), &self.root);
            }
            rel
        };
        if let Some(bk) = self.get_mut(id) {
            bk.formats.retain(|x| x.file != file);
        }
        self.log(
            format!(
                "Removed {} from \"{}\"",
                f.format.to_uppercase(),
                b.meta.title
            ),
            OpKind::RemoveFormat {
                id,
                format: f,
                trashed,
            },
        );
        self.save()
    }

    pub(crate) fn undo_remove_format(
        &mut self,
        id: u64,
        format: &FormatFile,
        trashed: &str,
    ) -> Result<()> {
        if !trashed.is_empty() {
            fsops::apply(&[FileMove::new(
                self.root.join(trashed),
                self.root.join(&format.file),
            )])?;
        }
        if let Some(b) = self.get_mut(id) {
            if !b.formats.iter().any(|f| f.file == format.file) {
                b.formats.push(format.clone());
            }
        }
        self.save()
    }

    /// Replaces the linked resources (audiobooks) of a book; one undo step.
    pub fn set_resources(
        &mut self,
        id: u64,
        resources: Vec<crate::formats::Resource>,
        label: String,
    ) -> Result<()> {
        self.writable()?;
        let b = self.get_mut(id).context("no such book")?;
        if b.resources == resources {
            return Ok(());
        }
        let before = std::mem::replace(&mut b.resources, resources);
        self.log(label, OpKind::SetResources { id, before });
        self.save()
    }

    /// The path of a format file of `b`.
    pub fn format_path(&self, f: &FormatFile) -> PathBuf {
        self.root.join(&f.file)
    }

    /// The best file of the book for `target` that exists on disk.
    pub fn best_existing(&self, b: &Book, target: Target) -> Option<PathBuf> {
        let all = b.all_formats();
        let p = self.root.join(&formats::choose(&all, target)?.file);
        p.exists().then_some(p)
    }
}
