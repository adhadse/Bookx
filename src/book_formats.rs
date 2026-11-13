//! Several formats in one book (roadmap PP-G-4/PP-G-6): merging books into one, offering to add a
//! file as another format of a book that is already in the library, and the rows of the
//! *Formats* list in the details panel. The file handling is `core::merge`; this is the glue with
//! the catalogue store (collections, reading status) and the pure texts for the windows.
//!
//! No GTK in here, so it is unit tested; the windows are in `ui/formats.rs`.

use std::path::PathBuf;

use crate::core::formats::{format_of, FormatFile, Target};
use crate::core::library::{Book, Library, Prepared};
use crate::core::merge::MergePlan;

/// A file that is another format of a book the library has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub existing: u64,
    pub existing_title: String,
    /// `azw3`, `mobi`, ...
    pub format: String,
    pub path: PathBuf,
}

/// The format of a file being imported (`kepub.epub` is `kepub`).
pub fn incoming_format(path: &std::path::Path) -> String {
    format_of(
        &path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
    )
}

/// Is the prepared book another format of one the library already has (same identifier, or same
/// title and author) in a format the library lacks? Then adding it as a format is offered.
pub fn offer_for(lib: &Library, p: &Prepared) -> Option<Offer> {
    let src = p.source_file.as_ref()?;
    let format = incoming_format(src);
    if format.is_empty() {
        return None;
    }
    let id = lib.find_same_book(&p.meta)?;
    let b = lib.get(id)?;
    if b.all_formats().iter().any(|f| f.format == format) {
        return None;
    }
    Some(Offer {
        existing: id,
        existing_title: b.meta.title.clone(),
        format,
        path: src.clone(),
    })
}

pub use bookx_store::formats::merge;

/// The lines of the preview: what the merged book will have, what is skipped and what is not kept.
pub fn preview_lines(plan: &MergePlan) -> Vec<String> {
    let mut v = vec![format!("Result: {}", plan.summary())];
    for i in &plan.items {
        match &i.blocked {
            None => v.push(format!(
                "Merge \"{}\" ({}, {})",
                i.title,
                i.formats.join(", "),
                crate::library_health::size_text(i.size)
            )),
            Some(why) => v.push(format!("Skip \"{}\": {why}", i.title)),
        }
    }
    v.extend(plan.notes.iter().cloned());
    v
}

/// The sentence shown instead of opening a protected book (`label` is the scheme: `Kindle`).
pub fn protected_message(label: &str) -> String {
    format!("Protected ({label}): Bookx cannot open it; Bookx does not remove DRM.")
}

/// One row of the *Formats* list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatRow {
    pub file: String,
    pub format: String,
    pub size: u64,
    pub main: bool,
    pub exists: bool,
}

impl FormatRow {
    /// Bookx's reader opens EPUB; the other formats go to the system's default application.
    pub fn opens_in_bookx(&self) -> bool {
        self.format == "epub" || self.format == "kepub"
    }
}

pub fn rows(lib: &Library, b: &Book) -> Vec<FormatRow> {
    b.all_formats()
        .into_iter()
        .enumerate()
        .map(|(i, f): (usize, FormatFile)| FormatRow {
            exists: lib.root.join(&f.file).exists(),
            main: i == 0,
            size: f.size,
            format: f.format,
            file: f.file,
        })
        .collect()
}

/// The format target for a kind of device: a Kobo gets KEPUB, a Kindle AZW3 or MOBI, the rest EPUB.
pub fn target_for_device(kind: Option<bookx_serve::devices::DeviceKind>) -> Target {
    use bookx_serve::devices::DeviceKind;
    match kind {
        Some(DeviceKind::Kobo) => Target::Kobo,
        Some(DeviceKind::Kindle) => Target::Kindle,
        _ => Target::Generic,
    }
}

/// The path to hand over from a book's format files `(absolute path, format)` for `target`.
pub fn pick_path(files: &[(PathBuf, FormatFile)], target: Target) -> PathBuf {
    let formats: Vec<FormatFile> = files.iter().map(|(_, f)| f.clone()).collect();
    crate::core::formats::choose(&formats, target)
        .and_then(|c| files.iter().find(|(_, f)| f.file == c.file))
        .or(files.first())
        .map(|(p, _)| p.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::AddOutcome;
    use bookx_store::SqliteStore;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-bf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn md(d: &std::path::Path, title: &str, author: &str) -> PathBuf {
        let p = d.join(format!("{title}.md"));
        std::fs::write(
            &p,
            format!("---\ntitle: \"{title}\"\nauthor: \"{author}\"\n---\n# One\n\nHi.\n"),
        )
        .unwrap();
        p
    }

    fn add(lib: &mut Library, d: &std::path::Path, title: &str, author: &str) -> u64 {
        match lib.add_path(&md(d, title, author), true).unwrap() {
            AddOutcome::Added(id) => id,
            AddOutcome::Duplicate(_) => panic!(),
        }
    }

    #[test]
    fn a_second_format_of_a_book_is_offered_but_the_same_format_is_not() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let id = add(&mut lib, &d, "Dune", "Frank Herbert");
        let mut p = crate::core::library::prepare_import(&md(&d, "Dune", "Frank Herbert")).unwrap();
        // an AZW3 file that was converted for import
        p.source_file = Some(d.join("Dune.azw3"));
        let o = offer_for(&lib, &p).unwrap();
        assert_eq!(o.existing, id);
        assert_eq!(o.format, "azw3");
        // an EPUB of the book is a plain duplicate: no offer
        p.source_file = Some(d.join("Dune.epub"));
        assert_eq!(offer_for(&lib, &p), None);
        // a different book, or no file, is no offer either
        let other = crate::core::library::prepare_import(&md(&d, "Emma", "Jane Austen")).unwrap();
        assert_eq!(offer_for(&lib, &other), None);
        p.source_file = None;
        assert_eq!(offer_for(&lib, &p), None);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn merging_keeps_collections_status_and_progress() {
        let d = tmp();
        let store = SqliteStore::open_in_memory().unwrap();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(&mut lib, &d, "Dune", "Frank Herbert");
        let b = add(&mut lib, &d, "Dune Azw", "Frank Herbert");
        let old = lib.get(b).unwrap().file.clone();
        let new = old.replace(".epub", ".azw3");
        std::fs::rename(lib.root.join(&old), lib.root.join(&new)).unwrap();
        lib.get_mut(b).unwrap().file = new;
        lib.get_mut(b).unwrap().progress = 0.6;
        lib.save().unwrap();
        store.put_many(&lib.books).unwrap();
        let c = store.create_collection("Sci-fi").unwrap();
        store.add_to_collection(c, b).unwrap();
        store
            .set_status(b, bookx_store::ReadingStatus::Reading)
            .unwrap();

        assert_eq!(merge(&mut lib, &store, a, &[b]).unwrap(), 1);
        assert_eq!(lib.books.len(), 1);
        assert_eq!(lib.get(a).unwrap().progress, 0.6);
        assert!(store.collections_of(a).contains(&c));
        assert_eq!(
            store.get_status(a),
            Some(bookx_store::ReadingStatus::Reading)
        );
        let rows = rows(&lib, lib.get(a).unwrap());
        assert_eq!(rows.len(), 2);
        assert!(rows[0].main && rows[0].opens_in_bookx());
        assert!(!rows[1].opens_in_bookx() && rows[1].exists);
        let b = lib.get(a).unwrap();
        let best = |t| {
            lib.best_existing(b, t)
                .unwrap()
                .to_string_lossy()
                .to_string()
        };
        assert!(best(Target::Kindle).ends_with(".azw3"));
        assert!(best(Target::Kobo).ends_with(".epub"));
        // a refused merge changes nothing
        let again = add(&mut lib, &d, "Another", "X");
        let before = store.get_status(a);
        assert!(merge(&mut lib, &store, a, &[again]).is_err());
        assert_eq!(store.get_status(a), before);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_protected_message_is_clear_and_promises_nothing_else() {
        let m = protected_message("Kindle");
        assert!(m.starts_with("Protected (Kindle)"));
        assert!(m.contains("cannot open it") && m.contains("does not remove DRM"));
    }

    #[test]
    fn a_device_gets_the_format_it_reads_best() {
        use bookx_serve::devices::DeviceKind;
        let f = |name: &str| {
            (
                PathBuf::from(format!("/lib/{name}")),
                FormatFile {
                    file: name.into(),
                    format: crate::core::formats::format_of(name),
                    size: 1,
                },
            )
        };
        let files = vec![f("a.epub"), f("a.kepub.epub"), f("a.azw3")];
        let pick = |k| pick_path(&files, target_for_device(k));
        assert_eq!(
            pick(Some(DeviceKind::Kobo)),
            PathBuf::from("/lib/a.kepub.epub")
        );
        assert_eq!(pick(Some(DeviceKind::Kindle)), PathBuf::from("/lib/a.azw3"));
        assert_eq!(
            pick(Some(DeviceKind::Generic)),
            PathBuf::from("/lib/a.epub")
        );
        assert_eq!(pick(None), PathBuf::from("/lib/a.epub"));
        assert_eq!(pick_path(&[], Target::Generic), PathBuf::new());
    }

    #[test]
    fn the_preview_says_what_is_merged_skipped_and_not_kept() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(&mut lib, &d, "One", "A");
        let b = add(&mut lib, &d, "Two", "A");
        let plan = lib.plan_merge(a, &[b]).unwrap();
        let lines = preview_lines(&plan);
        assert!(lines[0].starts_with("Result: One: epub"));
        assert!(lines[1].starts_with("Skip \"Two\": both have a EPUB"));
        assert!(lines.iter().any(|l| l.contains("Undo brings them back")));
        let _ = std::fs::remove_dir_all(d);
    }
}
