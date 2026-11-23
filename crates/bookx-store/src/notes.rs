//! Notes attached to an author, a series, a tag or a publisher (roadmap L20). Plain text, kept in
//! the catalogue database so they travel with the library, its snapshots and its backups.

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

use crate::store::SqliteStore;

/// What a note can be attached to.
pub const KINDS: [&str; 4] = ["author", "series", "tag", "publisher"];
/// The longest note, in characters.
pub const MAX_NOTE_CHARS: usize = 20_000;

pub(crate) fn migrate_v9(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS item_notes(
             kind TEXT NOT NULL,
             name TEXT NOT NULL COLLATE NOCASE,
             text TEXT NOT NULL,
             updated INTEGER NOT NULL,
             PRIMARY KEY(kind, name)
         ) WITHOUT ROWID;",
    )?;
    Ok(())
}

fn check_kind(kind: &str) -> Result<()> {
    if KINDS.contains(&kind) {
        Ok(())
    } else {
        bail!(
            "notes can be attached to {}, not `{kind}`",
            KINDS.join(", ")
        )
    }
}

impl SqliteStore {
    /// The note of an item, if it has one.
    pub fn item_note(&self, kind: &str, name: &str) -> Option<String> {
        self.lock()
            .query_row(
                "SELECT text FROM item_notes WHERE kind = ?1 AND name = ?2",
                params![kind, name.trim()],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Sets the note of an item; an empty text removes it.
    pub fn set_item_note(&self, kind: &str, name: &str, text: &str) -> Result<()> {
        check_kind(kind)?;
        let name = name.trim();
        if name.is_empty() {
            bail!("a note needs the name of what it is about");
        }
        if text.chars().count() > MAX_NOTE_CHARS {
            bail!("a note can have {MAX_NOTE_CHARS} characters at most");
        }
        let conn = self.lock();
        if text.trim().is_empty() {
            conn.execute(
                "DELETE FROM item_notes WHERE kind = ?1 AND name = ?2",
                params![kind, name],
            )?;
        } else {
            conn.execute(
                "INSERT INTO item_notes(kind, name, text, updated) VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(kind, name) DO UPDATE SET text = excluded.text, updated = excluded.updated",
                params![kind, name, text, chrono::Utc::now().timestamp()],
            )?;
        }
        Ok(())
    }

    /// Every note of a kind, by name.
    pub fn item_notes(&self, kind: &str) -> Vec<(String, String)> {
        let conn = self.lock();
        let Ok(mut st) =
            conn.prepare("SELECT name, text FROM item_notes WHERE kind = ?1 ORDER BY name")
        else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([kind], |r| Ok((r.get(0)?, r.get(1)?))) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    /// Moves the notes of a renamed item to its new name (an existing note of the new name wins).
    pub fn rename_item_note(&self, kind: &str, from: &str, to: &str) -> Result<()> {
        check_kind(kind)?;
        self.lock().execute(
            "UPDATE OR IGNORE item_notes SET name = ?3 WHERE kind = ?1 AND name = ?2",
            params![kind, from.trim(), to.trim()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_kept_per_kind_and_name_ignoring_case() {
        let s = SqliteStore::open_in_memory().unwrap();
        assert_eq!(s.item_note("author", "Ann"), None);
        s.set_item_note("author", " Ann ", "Met her at the fair")
            .unwrap();
        s.set_item_note("series", "Ann", "not the author").unwrap();
        assert_eq!(
            s.item_note("author", "ann").as_deref(),
            Some("Met her at the fair")
        );
        assert_eq!(
            s.item_note("series", "Ann").as_deref(),
            Some("not the author")
        );
        s.set_item_note("author", "ANN", "Changed").unwrap();
        assert_eq!(
            s.item_notes("author"),
            vec![("Ann".to_string(), "Changed".to_string())]
        );
        s.set_item_note("author", "Ann", "  ").unwrap();
        assert_eq!(
            s.item_note("author", "Ann"),
            None,
            "an empty note is removed"
        );
    }

    #[test]
    fn bad_input_is_refused_and_a_rename_moves_the_note() {
        let s = SqliteStore::open_in_memory().unwrap();
        assert!(s.set_item_note("book", "x", "y").is_err());
        assert!(s.set_item_note("tag", " ", "y").is_err());
        assert!(
            s.set_item_note("tag", "t", &"x".repeat(MAX_NOTE_CHARS + 1))
                .is_err()
        );
        s.set_item_note("tag", "old", "about old").unwrap();
        s.rename_item_note("tag", "old", "new").unwrap();
        assert_eq!(s.item_note("tag", "old"), None);
        assert_eq!(s.item_note("tag", "new").as_deref(), Some("about old"));
    }

    #[test]
    fn notes_survive_a_snapshot() {
        let dir = std::env::temp_dir().join(format!("bookx-notes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = SqliteStore::open_in_memory().unwrap();
        s.set_item_note("publisher", "Ace", "Paperbacks").unwrap();
        let f = dir.join("snap.db");
        s.backup_to(&f).unwrap();
        let t = SqliteStore::open(&f).unwrap();
        assert_eq!(
            t.item_note("publisher", "ace").as_deref(),
            Some("Paperbacks")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
