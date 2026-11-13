//! Transactional file moves: a plan is applied completely or not at all.
//!
//! Library files (a book, its cover, the other files of its folder) move together. If any step
//! fails, every step done so far is reversed, the folders created for the plan are removed again,
//! and the error says which move failed and whether the rollback was clean. Rename errors are
//! never ignored.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// One file to move. Both paths are absolute (or relative to the same base).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMove {
    pub from: PathBuf,
    pub to: PathBuf,
}

impl FileMove {
    pub fn new(from: impl Into<PathBuf>, to: impl Into<PathBuf>) -> FileMove {
        FileMove {
            from: from.into(),
            to: to.into(),
        }
    }
}

/// Moves a file: `rename`, or copy and delete when the two paths are on different file systems.
pub fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            std::fs::copy(from, to)?;
            if let Err(e) = std::fs::remove_file(from) {
                let _ = std::fs::remove_file(to);
                return Err(e);
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV is 18 on Linux and macOS
    e.raw_os_error() == Some(18)
}

/// Checks a plan without touching the disk: no source is missing, no target exists (unless another
/// move of the plan vacates it), no two moves share a target.
pub fn validate(moves: &[FileMove]) -> Result<()> {
    let sources: HashSet<&Path> = moves.iter().map(|m| m.from.as_path()).collect();
    let mut targets: HashSet<&Path> = HashSet::new();
    for m in moves {
        if !m.from.exists() {
            bail!("{} does not exist", m.from.display());
        }
        if !targets.insert(m.to.as_path()) {
            bail!("two files would be moved to {}", m.to.display());
        }
        if m.to.exists() && !sources.contains(m.to.as_path()) {
            bail!("{} already exists", m.to.display());
        }
    }
    Ok(())
}

/// Orders moves so that a target is free before something is moved onto it.
fn order(moves: &[FileMove]) -> Result<Vec<FileMove>> {
    let mut pending: Vec<FileMove> = moves.iter().filter(|m| m.from != m.to).cloned().collect();
    let mut done = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let blocked: HashSet<PathBuf> = pending.iter().map(|m| m.from.clone()).collect();
        let (ready, rest): (Vec<_>, Vec<_>) =
            pending.into_iter().partition(|m| !blocked.contains(&m.to));
        if ready.is_empty() {
            bail!("the planned moves form a cycle");
        }
        done.extend(ready);
        pending = rest;
    }
    Ok(done)
}

/// Applies `moves` with the default mover. See [`apply_with`].
pub fn apply(moves: &[FileMove]) -> Result<()> {
    apply_with(moves, &mut |a, b| move_file(a, b))
}

/// Applies `moves` using `mover` to move each file (tests inject failures here). On the first
/// error everything done is reversed; the error names the failing move.
pub fn apply_with(
    moves: &[FileMove],
    mover: &mut dyn FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<()> {
    validate(moves)?;
    let ordered = order(moves)?;
    let mut created: Vec<PathBuf> = Vec::new();
    let mut done: Vec<&FileMove> = Vec::new();
    let mut failure: Option<(usize, anyhow::Error)> = None;
    for (i, m) in ordered.iter().enumerate() {
        if let Some(parent) = m.to.parent() {
            match create_dirs(parent, &mut created) {
                Ok(()) => {}
                Err(e) => {
                    failure = Some((i, e.into()));
                    break;
                }
            }
        }
        match mover(&m.from, &m.to) {
            Ok(()) => done.push(m),
            Err(e) => {
                failure = Some((i, e.into()));
                break;
            }
        }
    }
    let Some((i, error)) = failure else {
        return Ok(());
    };
    let mut stuck = Vec::new();
    for m in done.iter().rev() {
        if let Err(e) = move_file(&m.to, &m.from) {
            stuck.push(format!("{}: {e}", m.to.display()));
        }
    }
    for d in created.iter().rev() {
        let _ = std::fs::remove_dir(d);
    }
    let failed = &ordered[i];
    if stuck.is_empty() {
        bail!(
            "could not move {} to {}: {error}. Nothing was changed.",
            failed.from.display(),
            failed.to.display()
        );
    }
    bail!(
        "could not move {} to {}: {error}. Undoing the earlier moves also failed for: {}",
        failed.from.display(),
        failed.to.display(),
        stuck.join("; ")
    );
}

/// `create_dir_all` that remembers which folders it made.
fn create_dirs(dir: &Path, created: &mut Vec<PathBuf>) -> io::Result<()> {
    if dir.as_os_str().is_empty() || dir.exists() {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        create_dirs(parent, created)?;
    }
    std::fs::create_dir(dir)?;
    created.push(dir.to_path_buf());
    Ok(())
}

/// Removes `dir` and its parents up to (not including) `stop` while they are empty.
pub fn prune_empty_dirs(dir: &Path, stop: &Path) {
    let mut cur = Some(dir.to_path_buf());
    while let Some(d) = cur {
        if d == stop || !d.starts_with(stop) {
            break;
        }
        // remove_dir only succeeds on an empty folder, which is exactly the point
        if std::fs::remove_dir(&d).is_err() {
            break;
        }
        cur = d.parent().map(Path::to_path_buf);
    }
}

/// Moves that undo `moves`.
pub fn reversed(moves: &[FileMove]) -> Vec<FileMove> {
    moves
        .iter()
        .map(|m| FileMove::new(m.to.clone(), m.from.clone()))
        .collect()
}

/// Number of moves per source folder; handy for previews.
pub fn count_by_dir(moves: &[FileMove]) -> HashMap<PathBuf, usize> {
    let mut h = HashMap::new();
    for m in moves {
        if let Some(p) = m.from.parent() {
            *h.entry(p.to_path_buf()).or_insert(0) += 1;
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-fsops-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn file(d: &Path, rel: &str, text: &str) -> PathBuf {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn moves_files_and_creates_folders() {
        let d = tmp();
        let a = file(&d, "a.txt", "A");
        let b = file(&d, "b.txt", "B");
        apply(&[
            FileMove::new(&a, d.join("x/y/a.txt")),
            FileMove::new(&b, d.join("x/b.txt")),
        ])
        .unwrap();
        assert!(!a.exists() && !b.exists());
        assert_eq!(std::fs::read_to_string(d.join("x/y/a.txt")).unwrap(), "A");
        assert_eq!(std::fs::read_to_string(d.join("x/b.txt")).unwrap(), "B");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_injected_failure_rolls_everything_back() {
        let d = tmp();
        let a = file(&d, "a.txt", "A");
        let b = file(&d, "b.txt", "B");
        let c = file(&d, "c.txt", "C");
        let plan = [
            FileMove::new(&a, d.join("new/a.txt")),
            FileMove::new(&b, d.join("new/deeper/b.txt")),
            FileMove::new(&c, d.join("new/c.txt")),
        ];
        let mut calls = 0;
        let err = apply_with(&plan, &mut |from, to| {
            calls += 1;
            if calls == 3 {
                return Err(io::Error::other("injected"));
            }
            move_file(from, to)
        })
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("injected") && msg.contains("Nothing was changed"),
            "{msg}"
        );
        for (p, text) in [(&a, "A"), (&b, "B"), (&c, "C")] {
            assert_eq!(std::fs::read_to_string(p).unwrap(), text, "{}", p.display());
        }
        assert!(!d.join("new").exists(), "created folders are removed again");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_failing_rollback_is_reported() {
        let d = tmp();
        let a = file(&d, "a.txt", "A");
        let b = file(&d, "b.txt", "B");
        let plan = [
            FileMove::new(&a, d.join("n/a.txt")),
            FileMove::new(&b, d.join("n/b.txt")),
        ];
        let mut calls = 0;
        let err = apply_with(&plan, &mut |from, to| {
            calls += 1;
            if calls == 2 {
                // the first file disappears, so it cannot be moved back
                std::fs::remove_file(d.join("n/a.txt")).unwrap();
                return Err(io::Error::other("boom"));
            }
            move_file(from, to)
        })
        .unwrap_err();
        assert!(format!("{err}").contains("Undoing the earlier moves also failed"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn plans_are_validated_before_anything_moves() {
        let d = tmp();
        let a = file(&d, "a.txt", "A");
        let b = file(&d, "b.txt", "B");
        assert!(apply(&[FileMove::new(&a, &b)]).is_err(), "target exists");
        assert!(apply(&[FileMove::new(d.join("none"), d.join("x"))]).is_err());
        assert!(apply(&[
            FileMove::new(&a, d.join("same")),
            FileMove::new(&b, d.join("same"))
        ])
        .is_err());
        assert!(a.exists() && b.exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_swap_chain_is_ordered_and_a_cycle_is_refused() {
        let d = tmp();
        let a = file(&d, "a", "A");
        let b = file(&d, "b", "B");
        apply(&[FileMove::new(&b, d.join("c")), FileMove::new(&a, &b)]).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("c")).unwrap(), "B");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "A");
        let x = file(&d, "x", "X");
        let y = file(&d, "y", "Y");
        assert!(apply(&[FileMove::new(&x, &y), FileMove::new(&y, &x)]).is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn empty_parents_are_pruned_up_to_the_stop_folder() {
        let d = tmp();
        std::fs::create_dir_all(d.join("a/b/c")).unwrap();
        prune_empty_dirs(&d.join("a/b/c"), &d);
        assert!(!d.join("a").exists());
        assert!(d.exists());
        file(&d, "k/keep.txt", "k");
        prune_empty_dirs(&d.join("k"), &d);
        assert!(d.join("k/keep.txt").exists());
        let _ = std::fs::remove_dir_all(d);
    }
}
