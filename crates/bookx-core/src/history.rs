//! The library's operation log and trash (roadmap PP-C-7 / PP-G-33).
//!
//! Every change of the library as a whole (import, delete, metadata edit, layout change) is one
//! [`Op`] in `<root>/.bookx/oplog.json`, with what is needed to take it back. Deleted books are not
//! erased: their files are moved to `<root>/.bookx-trash/<name>/files/…` together with an
//! `entry.json` holding the catalogue record, until the trash is emptied.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::library::Book;
use crate::opf::Metadata;

/// Folder for the small state files of the library (operation log, import queue).
pub const STATE_DIR: &str = ".bookx";
/// Folder holding deleted books.
pub const TRASH_DIR: &str = ".bookx-trash";
/// Operations kept in the log.
pub const MAX_OPS: usize = 200;

/// A file or folder of one book that moved, relative to the library folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookMove {
    pub id: u64,
    pub old_file: String,
    pub new_file: String,
    pub old_cover: Option<String>,
    pub new_cover: Option<String>,
    /// Every file that moved for this book: (from, to).
    pub files: Vec<(String, String)>,
}

/// What happened to the cover in an edit, so undo can put it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverUndo {
    /// The book had no cover before.
    Absent,
    /// The old cover is kept at this path (relative to the library folder).
    Saved(String),
    /// The old cover was larger than the backup limit and was not kept.
    TooBig,
}

/// One book that was folded into another by a merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedBook {
    /// Trash entry holding the catalogue record.
    pub trash: String,
    pub old_file: String,
    /// Where its format file is now (differs from `old_file` when it moved into the primary's folder).
    pub new_file: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OpKind {
    /// Books moved to the trash (entry names).
    Delete { trash: Vec<String> },
    /// Books added.
    Import { ids: Vec<u64> },
    /// The metadata of a book was edited; `before` is what it was. `cover` is set when the edit
    /// replaced or removed the cover.
    Edit {
        id: u64,
        before: Box<Metadata>,
        #[serde(default)]
        cover: Option<CoverUndo>,
    },
    /// Several books were edited as one step (rename an author everywhere); `before` per book.
    EditMany { edits: Vec<(u64, Box<Metadata>)> },
    /// Books were merged as formats of `primary`: the others left the catalogue (their records
    /// are in the trash) and their files became formats of the primary book.
    MergeFormats {
        primary: u64,
        before_file: String,
        before_formats: Vec<crate::formats::FormatFile>,
        merged: Vec<MergedBook>,
    },
    /// The linked resources (audiobooks) of a book changed; `before` is the earlier list.
    SetResources {
        id: u64,
        before: Vec<crate::formats::Resource>,
    },
    /// A format file was added to a book.
    AddFormat { id: u64, file: String },
    /// A format file was taken off a book; it waits in `trashed` (relative to the library).
    RemoveFormat {
        id: u64,
        format: crate::formats::FormatFile,
        trashed: String,
    },
    /// Files were moved to follow a layout change or a bulk rename.
    Move { moves: Vec<BookMove> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub seq: u64,
    pub at: DateTime<Utc>,
    pub label: String,
    pub kind: OpKind,
    #[serde(default)]
    pub undone: bool,
}

/// One deleted book in the trash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashEntry {
    pub name: String,
    pub at: DateTime<Utc>,
    pub book: Book,
    /// Paths relative to the library folder that were moved into the trash.
    #[serde(default)]
    pub files: Vec<String>,
}

pub fn oplog_path(root: &Path) -> PathBuf {
    root.join(STATE_DIR).join("oplog.json")
}

pub fn trash_dir(root: &Path) -> PathBuf {
    root.join(TRASH_DIR)
}

/// Reads the log; a missing or damaged file gives an empty log.
pub fn load_ops(root: &Path) -> Vec<Op> {
    std::fs::read_to_string(oplog_path(root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_ops(root: &Path, ops: &[Op]) -> std::io::Result<()> {
    let p = oplog_path(root);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(ops).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, &p)
}

/// Adds `op` to `ops`: an import right after another import (within seconds) joins it, so an
/// import of fifty books is one undo step. Keeps at most [`MAX_OPS`].
pub fn push_op(ops: &mut Vec<Op>, label: String, kind: OpKind, at: DateTime<Utc>) {
    if let OpKind::Import { ids } = &kind {
        if let Some(last) = ops.last_mut() {
            if let OpKind::Import { ids: prev } = &mut last.kind {
                if !last.undone && (at - last.at).num_seconds().abs() < 10 {
                    prev.extend(ids.iter().copied());
                    last.at = at;
                    last.label = format!("Imported {} books", prev.len());
                    return;
                }
            }
        }
    }
    let seq = ops.last().map(|o| o.seq + 1).unwrap_or(1);
    ops.push(Op {
        seq,
        at,
        label,
        kind,
        undone: false,
    });
    if ops.len() > MAX_OPS {
        let n = ops.len() - MAX_OPS;
        ops.drain(..n);
    }
}

/// The newest operation that can still be undone.
pub fn last_undoable(ops: &[Op]) -> Option<&Op> {
    ops.iter().rev().find(|o| !o.undone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn imports_in_a_row_join_into_one_step() {
        let mut ops = Vec::new();
        let t = now();
        push_op(
            &mut ops,
            "Imported A".into(),
            OpKind::Import { ids: vec![1] },
            t,
        );
        push_op(
            &mut ops,
            "Imported B".into(),
            OpKind::Import { ids: vec![2] },
            t + Duration::seconds(2),
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind, OpKind::Import { ids: vec![1, 2] });
        assert_eq!(ops[0].label, "Imported 2 books");
        push_op(
            &mut ops,
            "Imported C".into(),
            OpKind::Import { ids: vec![3] },
            t + Duration::seconds(60),
        );
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[1].seq, 2);
    }

    #[test]
    fn an_undone_import_does_not_swallow_the_next_one() {
        let mut ops = Vec::new();
        let t = now();
        push_op(
            &mut ops,
            "Imported A".into(),
            OpKind::Import { ids: vec![1] },
            t,
        );
        ops[0].undone = true;
        push_op(
            &mut ops,
            "Imported B".into(),
            OpKind::Import { ids: vec![2] },
            t,
        );
        assert_eq!(ops.len(), 2);
        assert_eq!(last_undoable(&ops).unwrap().seq, 2);
    }

    #[test]
    fn the_log_is_bounded() {
        let mut ops = Vec::new();
        for i in 0..(MAX_OPS + 30) {
            push_op(
                &mut ops,
                format!("d{i}"),
                OpKind::Delete { trash: vec![] },
                now(),
            );
        }
        assert_eq!(ops.len(), MAX_OPS);
        assert_eq!(ops.last().unwrap().seq as usize, MAX_OPS + 30);
    }

    #[test]
    fn the_log_round_trips_through_the_file() {
        let d = std::env::temp_dir().join(format!("bookx-oplog-{}", uuid::Uuid::new_v4()));
        let mut ops = Vec::new();
        push_op(
            &mut ops,
            "x".into(),
            OpKind::Delete {
                trash: vec!["a".into()],
            },
            now(),
        );
        save_ops(&d, &ops).unwrap();
        assert_eq!(load_ops(&d), ops);
        std::fs::write(oplog_path(&d), "not json").unwrap();
        assert!(load_ops(&d).is_empty(), "a damaged log is ignored");
        let _ = std::fs::remove_dir_all(d);
    }
}
