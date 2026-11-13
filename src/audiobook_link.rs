//! Audiobook pairing (roadmap PP-G-8): finds `.m4b` files and folders of audio parts next to the
//! books of the library (and in folders the user names), pairs them by identifier, title and
//! author and links them to the book as an `audiobook` resource. Bookx plays nothing: *Open* hands
//! the item to the system's default player. The scanning and pairing is `bookx_sources::audiobooks`.
//!
//! No GTK in here, so it is unit tested; the window is `ui/audiobooks.rs`.

use crate::i18n::trf;
use std::path::{Path, PathBuf};

use anyhow::Result;
use bookx_sources::audiobooks::{pair, scan, Candidate, Pairing};

use crate::core::formats::{Resource, AUDIOBOOK};
use crate::core::library::{Book, Library};

/// How deep the folders are searched.
pub const DEPTH: usize = 5;

/// A proposed link, for the preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub book: u64,
    pub book_title: String,
    pub candidate: Candidate,
    pub reason: &'static str,
}

/// Looks in the library folder and in `extra` folders and pairs what it finds with books that do
/// not have an audiobook yet.
pub fn find(lib: &Library, extra: &[PathBuf]) -> Vec<Proposal> {
    let mut cands: Vec<Candidate> = scan(&lib.root, DEPTH);
    for d in extra {
        for c in scan(d, DEPTH) {
            if !cands.iter().any(|x| x.path == c.path) {
                cands.push(c);
            }
        }
    }
    let books: Vec<Book> = lib
        .books
        .iter()
        .filter(|b| !b.resources.iter().any(|r| r.kind == AUDIOBOOK))
        .cloned()
        .collect();
    let dir_of = |b: &Book| -> Option<PathBuf> { lib.book_dir_of(b).map(|d| lib.root.join(d)) };
    let pairs: Vec<Pairing> = pair(&books, &cands, &dir_of);
    pairs
        .into_iter()
        .filter_map(|p| {
            let b = books.iter().find(|b| b.id == p.book)?;
            Some(Proposal {
                book: p.book,
                book_title: b.meta.title.clone(),
                candidate: cands[p.candidate].clone(),
                reason: p.reason,
            })
        })
        .collect()
}

/// The resource for a path.
pub fn resource_for(path: &Path) -> Resource {
    Resource {
        kind: AUDIOBOOK.into(),
        path: path.to_string_lossy().to_string(),
        label: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
    }
}

/// Links the proposals to their books; every book is one undo step. Returns the number linked.
pub fn link(lib: &mut Library, proposals: &[Proposal]) -> Result<usize> {
    let mut n = 0;
    for p in proposals {
        let Some(b) = lib.get(p.book) else { continue };
        let mut res = b.resources.clone();
        res.push(resource_for(&p.candidate.path));
        lib.set_resources(
            p.book,
            res,
            trf("Linked audiobook to \"{}\"", &[&p.book_title]),
        )?;
        n += 1;
    }
    Ok(n)
}

/// Takes the link off (the files stay where they are).
pub fn unlink(lib: &mut Library, book: u64, path: &str) -> Result<()> {
    let b = lib
        .get(book)
        .ok_or_else(|| anyhow::anyhow!("no such book"))?;
    let title = b.meta.title.clone();
    let res: Vec<Resource> = b
        .resources
        .iter()
        .filter(|r| r.path != path)
        .cloned()
        .collect();
    lib.set_resources(book, res, trf("Unlinked audiobook from \"{}\"", &[&title]))
}

/// What the Open button shows: the item, whether it is still there.
pub fn describe(r: &Resource) -> String {
    let p = Path::new(&r.path);
    if !p.exists() {
        return format!("{} (missing)", r.label);
    }
    if p.is_dir() {
        let n = std::fs::read_dir(p)
            .map(|d| d.flatten().count())
            .unwrap_or(0);
        format!("{} · folder, {n} files", r.label)
    } else {
        format!(
            "{} · {}",
            r.label,
            crate::library_health::size_text(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::AddOutcome;

    fn setup() -> (Library, PathBuf) {
        let d = std::env::temp_dir().join(format!("bookx-al-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        for (t, a) in [("Dune", "Frank Herbert"), ("Emma", "Jane Austen")] {
            let p = d.join(format!("{t}.md"));
            std::fs::write(
                &p,
                format!("---\ntitle: \"{t}\"\nauthor: \"{a}\"\n---\n# One\n\nHi.\n"),
            )
            .unwrap();
            let AddOutcome::Added(_) = lib.add_path(&p, true).unwrap() else {
                panic!()
            };
        }
        (lib, d)
    }

    #[test]
    fn audio_in_the_book_folder_or_named_like_the_book_is_paired_and_linked_with_undo() {
        let (mut lib, d) = setup();
        // next to the book (its own folder)
        let dune_dir = lib.root.join(lib.book_dir_of(lib.get(1).unwrap()).unwrap());
        std::fs::write(dune_dir.join("audio.m4b"), vec![0u8; 50]).unwrap();
        // elsewhere, found by name
        let extra = d.join("audio");
        std::fs::create_dir_all(extra.join("Jane Austen")).unwrap();
        std::fs::write(extra.join("Jane Austen/Emma.m4b"), vec![0u8; 20]).unwrap();
        std::fs::write(extra.join("Unrelated Thing.m4b"), vec![0u8; 20]).unwrap();

        let found = find(&lib, std::slice::from_ref(&extra));
        assert_eq!(found.len(), 2, "{found:?}");
        let dune = found.iter().find(|p| p.book == 1).unwrap();
        assert_eq!(dune.reason, "in the book's folder");
        let emma = found.iter().find(|p| p.book == 2).unwrap();
        assert_eq!(emma.reason, "same title and author");

        assert_eq!(link(&mut lib, &found).unwrap(), 2);
        let r = &lib.get(2).unwrap().resources;
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, "audiobook");
        assert!(describe(&r[0]).starts_with("Emma.m4b · "));
        // linked books are not proposed again
        assert!(find(&lib, std::slice::from_ref(&extra)).is_empty());
        lib.undo_last().unwrap();
        assert!(
            lib.get(2).unwrap().resources.is_empty() || lib.get(1).unwrap().resources.is_empty()
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn unlinking_keeps_the_files_and_a_missing_item_is_called_missing() {
        let (mut lib, d) = setup();
        let f = d.join("custom.m4b");
        std::fs::write(&f, b"x").unwrap();
        lib.set_resources(1, vec![resource_for(&f)], "link".into())
            .unwrap();
        unlink(&mut lib, 1, &f.to_string_lossy()).unwrap();
        assert!(lib.get(1).unwrap().resources.is_empty());
        assert!(f.exists());
        assert!(describe(&resource_for(&d.join("gone.m4b"))).ends_with("(missing)"));
        let _ = std::fs::remove_dir_all(d);
    }
}
