//! Where the fields of a book came from (roadmap PP-G-1): the file itself, a metadata source such
//! as Open Library, or the user. Kept in its own table so the book record stays unchanged.

use anyhow::Result;
use rusqlite::params;

use crate::store::SqliteStore;

/// One recorded origin of a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSource {
    pub field: String,
    pub source: String,
    /// Unix seconds.
    pub at: i64,
}

impl SqliteStore {
    /// Records that `fields` of `book` now come from `source` (replacing earlier entries).
    pub fn set_provenance(&self, book: u64, fields: &[&str], source: &str, at: i64) -> Result<()> {
        let id = i64::try_from(book)?;
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for f in fields {
            tx.prepare_cached(
                "INSERT INTO provenance(book, field, source, at) VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(book, field) DO UPDATE SET source = excluded.source, at = excluded.at",
            )?
            .execute(params![id, f, source, at])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Where each recorded field of `book` came from, sorted by field name.
    pub fn provenance(&self, book: u64) -> Vec<FieldSource> {
        let Ok(id) = i64::try_from(book) else {
            return Vec::new();
        };
        let conn = self.lock();
        let Ok(mut st) = conn.prepare_cached(
            "SELECT field, source, at FROM provenance WHERE book = ?1 ORDER BY field",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([id], |r| {
            Ok(FieldSource {
                field: r.get(0)?,
                source: r.get(1)?,
                at: r.get(2)?,
            })
        }) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    /// Forgets the recorded origin of the given fields (for example after the user edits them).
    pub fn clear_provenance(&self, book: u64, fields: &[&str]) -> Result<()> {
        let id = i64::try_from(book)?;
        let conn = self.lock();
        for f in fields {
            conn.prepare_cached("DELETE FROM provenance WHERE book = ?1 AND field = ?2")?
                .execute(params![id, f])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::library::Book;

    fn store_with_book() -> SqliteStore {
        let s = SqliteStore::open_in_memory().unwrap();
        let b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":["A"],"author_sort":"A","publisher":"","language":"","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        s.put_many(&[b]).unwrap();
        s
    }

    #[test]
    fn sources_are_recorded_replaced_and_cleared() {
        let s = store_with_book();
        s.set_provenance(1, &["title", "publisher"], "file", 100)
            .unwrap();
        s.set_provenance(1, &["publisher"], "Open Library", 200)
            .unwrap();
        let p = s.provenance(1);
        assert_eq!(p.len(), 2);
        assert_eq!(
            p[0],
            FieldSource {
                field: "publisher".into(),
                source: "Open Library".into(),
                at: 200
            }
        );
        assert_eq!(p[1].source, "file");
        s.clear_provenance(1, &["publisher"]).unwrap();
        assert_eq!(s.provenance(1).len(), 1);
        assert!(s.provenance(99).is_empty());
    }

    #[test]
    fn deleting_a_book_deletes_its_provenance() {
        let s = store_with_book();
        s.set_provenance(1, &["title"], "file", 1).unwrap();
        bookx_core::contracts::Store::remove(&s, 1).unwrap();
        assert!(s.provenance(1).is_empty());
    }
}
