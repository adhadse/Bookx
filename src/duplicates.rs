//! Finding and merging duplicate books (roadmap L7), on top of `bookx-sources::dedup`.
//! No GTK in here, so it is unit tested.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use bookx_sources::dedup::{find_duplicates, merge_metadata};

use crate::core::library::{Book, Library};
use crate::core::opf::Metadata;

/// Books scoring at least this are treated as the same book.
pub const THRESHOLD: f64 = 0.9;

/// Books that look like the same one, with the copy that is kept by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub ids: Vec<u64>,
    pub keep: u64,
}

/// What the reader has already invested in a book (higher = more worth keeping).
fn investment(b: &Book) -> (u64, usize, u64) {
    (
        (b.progress * 1000.0) as u64,
        b.highlights.len() + b.bookmarks.len(),
        b.size,
    )
}

/// All duplicate groups. The copy to keep is the one with the most reading done, then the most
/// notes, then the biggest file, then the oldest.
pub fn find(books: &[Book]) -> Vec<Group> {
    let mut groups: Vec<Group> = find_duplicates(books, THRESHOLD)
        .into_iter()
        .filter(|s| s.ids.len() > 1)
        .filter_map(|s| {
            let keep = s
                .ids
                .iter()
                .filter_map(|id| books.iter().find(|b| b.id == *id))
                .max_by(|a, b| {
                    investment(a).cmp(&investment(b)).then(b.id.cmp(&a.id)) // the lower id wins a tie
                })?
                .id;
            Some(Group { ids: s.ids, keep })
        })
        .collect();
    groups.sort_by_key(|g| g.ids[0]);
    groups
}

/// Pairs of books the user said are *not* the same book (roadmap PP-G-4: a "not a duplicate"
/// memory). Kept in `<library>/.bookx/not-duplicates.json`; a damaged or missing file means none.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Apart {
    #[serde(default)]
    pairs: BTreeSet<(u64, u64)>,
}

fn pair(a: u64, b: u64) -> (u64, u64) {
    (a.min(b), a.max(b))
}

pub fn apart_path(root: &Path) -> PathBuf {
    root.join(crate::core::history::STATE_DIR)
        .join("not-duplicates.json")
}

impl Apart {
    pub fn load(root: &Path) -> Apart {
        std::fs::read(apart_path(root))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let p = apart_path(root);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }

    /// Remembers that none of `ids` is a copy of another.
    pub fn keep_apart(&mut self, ids: &[u64]) {
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                if a != b {
                    self.pairs.insert(pair(*a, *b));
                }
            }
        }
    }

    pub fn is_apart(&self, a: u64, b: u64) -> bool {
        self.pairs.contains(&pair(a, b))
    }

    /// Hides the groups whose every pair was declared different. A group in which some pair is still
    /// open stays whole: showing a copy too many is better than hiding a real one.
    pub fn hide(&self, groups: Vec<Group>) -> Vec<Group> {
        if self.pairs.is_empty() {
            return groups;
        }
        groups
            .into_iter()
            .filter(|g| {
                !g.ids
                    .iter()
                    .enumerate()
                    .all(|(i, a)| g.ids[i + 1..].iter().all(|b| self.is_apart(*a, *b)))
            })
            .collect()
    }
}

impl Group {
    /// True when the copies are different formats of the same book (no two share a format), so
    /// they can be merged as formats of one book instead of one being deleted.
    pub fn differ_in_format(&self, books: &[Book]) -> bool {
        let mut seen: Vec<String> = Vec::new();
        for id in &self.ids {
            let Some(b) = books.iter().find(|b| b.id == *id) else {
                return false;
            };
            for f in b.all_formats() {
                if seen.contains(&f.format) {
                    return false;
                }
                seen.push(f.format);
            }
        }
        self.ids.len() > 1
    }
}

/// One line describing a copy.
pub fn describe(b: &Book) -> String {
    let mb = b.size as f64 / 1024.0 / 1024.0;
    format!(
        "{} · {:.1} MB · {:.0}% read{}",
        b.meta.authors.join(" & "),
        mb,
        b.progress * 100.0,
        if b.highlights.is_empty() {
            String::new()
        } else {
            format!(" · {} highlights", b.highlights.len())
        }
    )
}

/// Merges `others` into `keep`: the metadata is combined (the kept book wins every field it has),
/// the furthest reading position is taken, and highlights and bookmarks are united.
pub fn merge_state(keep: &mut Book, others: &[Book]) {
    let metas: Vec<Metadata> = others.iter().map(|b| b.meta.clone()).collect();
    keep.meta = merge_metadata(&keep.meta, &metas);
    for o in others {
        if o.progress > keep.progress {
            keep.progress = o.progress;
            keep.position = o.position;
        }
        keep.last_read = keep.last_read.max(o.last_read);
        let hl: HashSet<String> = keep.highlights.iter().map(|h| h.id.clone()).collect();
        for h in &o.highlights {
            if !hl.contains(&h.id) {
                keep.highlights.push(h.clone());
            }
        }
        let bm: HashSet<String> = keep.bookmarks.iter().map(|b| b.id.clone()).collect();
        for b in &o.bookmarks {
            if !bm.contains(&b.id) {
                keep.bookmarks.push(b.clone());
            }
        }
    }
}

/// Merges the books `ids` of `lib` into `keep` and removes the others (with their files).
/// Returns how many books were removed.
pub fn merge(lib: &mut Library, ids: &[u64], keep: u64) -> Result<usize> {
    if !ids.contains(&keep) {
        bail!("the book to keep is not part of the group");
    }
    let others: Vec<Book> = ids
        .iter()
        .filter(|i| **i != keep)
        .filter_map(|i| lib.get(*i).cloned())
        .collect();
    if others.is_empty() {
        return Ok(0);
    }
    let Some(mut kept) = lib.get(keep).cloned() else {
        bail!("book {keep} is not in the library");
    };
    merge_state(&mut kept, &others);
    // the combined metadata goes into the book file itself (and may re-file it)
    lib.update_metadata(keep, kept.meta.clone())?;
    if let Some(b) = lib.get_mut(keep) {
        b.progress = kept.progress;
        b.position = kept.position;
        b.last_read = kept.last_read;
        b.highlights = kept.highlights;
        b.bookmarks = kept.bookmarks;
    }
    let remove: Vec<u64> = others.iter().map(|b| b.id).collect();
    lib.remove(&remove, true)?;
    lib.save()?;
    Ok(remove.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::convert::{create_epub, Chapter};
    use crate::core::library::{AddOutcome, Bookmark, Highlight, ReadPos};
    use chrono::Utc;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-dups-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn epub(
        dir: &std::path::Path,
        file: &str,
        title: &str,
        author: &str,
        body: &str,
    ) -> std::path::PathBuf {
        let md = Metadata {
            title: title.into(),
            authors: vec![author.into()],
            ..Default::default()
        };
        let ch = Chapter {
            title: "One".into(),
            body: format!("<p>{body}</p>"),
        };
        let p = dir.join(file);
        std::fs::write(&p, create_epub(&md, &[ch], None).to_bytes().unwrap()).unwrap();
        p
    }

    fn add(lib: &mut Library, p: &std::path::Path) -> u64 {
        match lib.add_path(p, true).unwrap() {
            AddOutcome::Added(id) | AddOutcome::Duplicate(id) => id,
        }
    }

    fn hl(id: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start: 0,
            end: 3,
            color: "yellow".into(),
            style: String::new(),
            note: String::new(),
            text: "abc".into(),
            created: Utc::now(),
        }
    }

    #[test]
    fn same_title_and_author_form_a_group_and_others_do_not() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(
            &mut lib,
            &epub(&d, "1.epub", "Dune", "Frank Herbert", "one"),
        );
        let b = add(
            &mut lib,
            &epub(&d, "2.epub", "Dune", "Herbert, Frank", "two"),
        );
        let c = add(
            &mut lib,
            &epub(&d, "3.epub", "Emma", "Jane Austen", "three"),
        );
        let groups = find(&lib.books);
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].ids, vec![a, b]);
        assert!(!groups[0].ids.contains(&c));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn copies_in_different_formats_offer_a_format_merge_and_same_format_copies_do_not() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(
            &mut lib,
            &epub(&d, "1.epub", "Dune", "Frank Herbert", "one"),
        );
        let b = add(
            &mut lib,
            &epub(&d, "2.epub", "Dune", "Frank Herbert", "two"),
        );
        let g = &find(&lib.books)[0];
        assert!(
            !g.differ_in_format(&lib.books),
            "two EPUBs are plain copies"
        );
        let old = lib.get(b).unwrap().file.clone();
        let new = old.replace(".epub", ".azw3");
        std::fs::rename(lib.root.join(&old), lib.root.join(&new)).unwrap();
        lib.get_mut(b).unwrap().file = new;
        let g = &find(&lib.books)[0];
        assert!(g.differ_in_format(&lib.books));
        let store = bookx_store::SqliteStore::open_in_memory().unwrap();
        assert_eq!(
            crate::book_formats::merge(&mut lib, &store, a, &[b]).unwrap(),
            1
        );
        assert!(find(&lib.books).is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn declared_apart_books_leave_the_group_and_the_memory_is_saved() {
        let g = |ids: &[u64]| Group {
            ids: ids.to_vec(),
            keep: ids[0],
        };
        let mut a = Apart::default();
        assert_eq!(
            a.hide(vec![g(&[1, 2])]),
            vec![g(&[1, 2])],
            "nothing remembered"
        );
        a.keep_apart(&[1, 2]);
        assert!(a.is_apart(2, 1) && !a.is_apart(1, 3));
        assert!(
            a.hide(vec![g(&[1, 2])]).is_empty(),
            "a pair that was declared apart is gone"
        );
        // of three copies, one open pair keeps the group on screen
        assert_eq!(a.hide(vec![g(&[1, 2, 3])]), vec![g(&[1, 2, 3])]);
        a.keep_apart(&[1, 2, 3]);
        assert!(a.hide(vec![g(&[1, 2, 3])]).is_empty());
        let dir = tmp();
        a.save(&dir).unwrap();
        assert_eq!(Apart::load(&dir), a);
        std::fs::write(apart_path(&dir), "garbage").unwrap();
        assert_eq!(
            Apart::load(&dir),
            Apart::default(),
            "a damaged file means none"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_library_without_copies_has_no_groups() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        add(&mut lib, &epub(&d, "1.epub", "Alpha", "A One", "x"));
        add(&mut lib, &epub(&d, "2.epub", "Beta", "B Two", "y"));
        assert!(find(&lib.books).is_empty());
        assert!(find(&[]).is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_copy_with_most_reading_is_kept_then_notes_then_size_then_age() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(&mut lib, &epub(&d, "1.epub", "Same", "Ann Author", "one"));
        let b = add(&mut lib, &epub(&d, "2.epub", "Same", "Ann Author", "two"));
        let c = add(&mut lib, &epub(&d, "3.epub", "Same", "Ann Author", "three"));
        // all equal: the oldest (lowest id) wins
        for bk in lib.books.iter_mut() {
            bk.size = 100;
        }
        assert_eq!(find(&lib.books)[0].keep, a);
        // the biggest file wins next
        lib.get_mut(b).unwrap().size = 500;
        assert_eq!(find(&lib.books)[0].keep, b);
        // notes beat size
        lib.get_mut(c).unwrap().highlights.push(hl("h"));
        assert_eq!(find(&lib.books)[0].keep, c);
        // reading progress beats everything
        lib.get_mut(a).unwrap().progress = 0.5;
        assert_eq!(find(&lib.books)[0].keep, a);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn merge_state_unites_metadata_notes_and_progress() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(&mut lib, &epub(&d, "1.epub", "Same", "Ann Author", "one"));
        let b = add(&mut lib, &epub(&d, "2.epub", "Same", "Ann Author", "two"));
        {
            let x = lib.get_mut(a).unwrap();
            x.meta.tags = vec!["fiction".into()];
            x.highlights.push(hl("h1"));
        }
        {
            let y = lib.get_mut(b).unwrap();
            y.meta.tags = vec!["Fiction".into(), "classic".into()];
            y.meta.publisher = "Pub".into();
            y.progress = 0.6;
            y.position = Some(ReadPos {
                spine: 2,
                frac: 0.1,
            });
            y.highlights.push(hl("h1"));
            y.highlights.push(hl("h2"));
            y.bookmarks.push(Bookmark {
                id: "b1".into(),
                spine: 1,
                frac: 0.2,
                label: "m".into(),
                created: Utc::now(),
            });
        }
        let mut keep = lib.get(a).unwrap().clone();
        let other = lib.get(b).unwrap().clone();
        merge_state(&mut keep, &[other]);
        assert_eq!(keep.progress, 0.6);
        assert_eq!(keep.position.unwrap().spine, 2);
        assert_eq!(keep.highlights.len(), 2, "h1 is not duplicated");
        assert_eq!(keep.bookmarks.len(), 1);
        assert_eq!(keep.meta.publisher, "Pub", "empty fields are filled");
        assert_eq!(
            keep.meta.tags.len(),
            2,
            "tags are united without case duplicates: {:?}",
            keep.meta.tags
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn merging_removes_the_others_and_their_files_and_keeps_the_state() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(&mut lib, &epub(&d, "1.epub", "Same", "Ann Author", "one"));
        let b = add(&mut lib, &epub(&d, "2.epub", "Same", "Ann Author", "two"));
        let c = add(
            &mut lib,
            &epub(&d, "3.epub", "Other", "Bob Writer", "three"),
        );
        lib.get_mut(b).unwrap().progress = 0.8;
        lib.get_mut(b).unwrap().highlights.push(hl("hb"));
        let b_path = lib.book_path(lib.get(b).unwrap());
        let n = merge(&mut lib, &[a, b], a).unwrap();
        assert_eq!(n, 1);
        assert!(lib.get(b).is_none());
        assert!(lib.get(c).is_some(), "unrelated books are untouched");
        assert!(!b_path.exists(), "the removed copy's file is deleted");
        let kept = lib.get(a).unwrap();
        assert_eq!(kept.progress, 0.8);
        assert_eq!(kept.highlights.len(), 1);
        assert!(lib.book_path(kept).exists());
        assert!(find(&lib.books).is_empty(), "no group is left");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn merge_refuses_a_keep_outside_the_group_and_handles_single_books() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(&mut lib, &epub(&d, "1.epub", "Same", "Ann Author", "one"));
        let b = add(&mut lib, &epub(&d, "2.epub", "Same", "Ann Author", "two"));
        assert!(merge(&mut lib, &[a, b], 999).is_err());
        assert_eq!(lib.books.len(), 2);
        // a group of one: nothing to do
        assert_eq!(merge(&mut lib, &[a], a).unwrap(), 0);
        assert_eq!(lib.books.len(), 2);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn describe_names_author_size_progress_and_notes() {
        let d = tmp();
        let mut lib = Library::in_memory(&d.join("lib")).unwrap();
        let a = add(&mut lib, &epub(&d, "1.epub", "Same", "Ann Author", "one"));
        let b = lib.get_mut(a).unwrap();
        b.progress = 0.42;
        b.size = 3 * 1024 * 1024;
        b.highlights.push(hl("x"));
        let s = describe(b);
        assert!(
            s.contains("Ann Author")
                && s.contains("3.0 MB")
                && s.contains("42% read")
                && s.contains("1 highlights"),
            "{s}"
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
