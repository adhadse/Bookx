//! Editions of one work (roadmap PP-G-5): explicit links between books that are editions or formats
//! of the same work, the books the user took out of an automatic grouping, and the edition that
//! opens first. Nothing here touches files or merges books: it is only a catalogue of "these books
//! belong together". The automatic suggestions (same title and author, same ISBN) are computed by
//! the app from the books and never stored; only what the user decided is.

use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use crate::store::SqliteStore;

/// One row of the `work_links` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkLink {
    pub book: u64,
    /// The work this book belongs to (the smallest book id when the link was made).
    pub work: u64,
    /// The edition that opens first and stands for the work in the grid.
    pub preferred: bool,
}

pub(crate) fn migrate_v7(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS work_links(
             book INTEGER PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
             work INTEGER NOT NULL,
             preferred INTEGER NOT NULL DEFAULT 0
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS work_links_work ON work_links(work);
         CREATE TABLE IF NOT EXISTS work_apart(
             book INTEGER PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE
         ) WITHOUT ROWID;",
    )?;
    Ok(())
}

fn id(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| anyhow::anyhow!("book id {v} is too large for the store"))
}

/// Drops links that are alone in their work (a link needs two books).
fn prune_lone(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM work_links WHERE work IN
           (SELECT work FROM work_links GROUP BY work HAVING COUNT(*) < 2)",
        [],
    )?;
    Ok(())
}

impl SqliteStore {
    /// Every explicit link, ordered by work then book.
    pub fn work_links(&self) -> Vec<WorkLink> {
        let conn = self.lock();
        let Ok(mut st) =
            conn.prepare("SELECT book, work, preferred FROM work_links ORDER BY work, book")
        else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([], |r| {
            Ok(WorkLink {
                book: r.get::<_, i64>(0)? as u64,
                work: r.get::<_, i64>(1)? as u64,
                preferred: r.get::<_, i64>(2)? != 0,
            })
        }) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    /// The books the user took out of the automatic grouping.
    pub fn work_apart(&self) -> Vec<u64> {
        let conn = self.lock();
        let Ok(mut st) = conn.prepare("SELECT book FROM work_apart ORDER BY book") else {
            return Vec::new();
        };
        let Ok(rows) = st.query_map([], |r| r.get::<_, i64>(0)) else {
            return Vec::new();
        };
        rows.flatten().map(|i| i as u64).collect()
    }

    /// Links books as editions of one work and returns the work id. Works the books already
    /// belong to are merged into one (their other members come along); books that were taken out
    /// of the automatic grouping are taken back in. Needs at least two books.
    pub fn link_editions(&self, books: &[u64]) -> Result<u64> {
        let mut ids: Vec<u64> = books.to_vec();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() < 2 {
            bail!("linking needs at least two books");
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for b in &ids {
            let exists: i64 =
                tx.query_row("SELECT COUNT(*) FROM books WHERE id = ?1", [id(*b)?], |r| {
                    r.get(0)
                })?;
            if exists == 0 {
                bail!("book {b} is not in the library");
            }
        }
        let mut works: Vec<i64> = Vec::new();
        for b in &ids {
            if let Ok(w) = tx.query_row(
                "SELECT work FROM work_links WHERE book = ?1",
                [id(*b)?],
                |r| r.get::<_, i64>(0),
            ) {
                works.push(w);
            }
        }
        works.sort_unstable();
        works.dedup();
        let target = match works.first() {
            Some(w) => *w,
            None => id(ids[0])?,
        };
        // keep one preferred edition: the one of the target work, else of the first merged work
        let mut keep_pref: Option<i64> = None;
        for w in &works {
            keep_pref = tx
                .query_row(
                    "SELECT book FROM work_links WHERE work = ?1 AND preferred = 1",
                    [w],
                    |r| r.get(0),
                )
                .ok();
            if keep_pref.is_some() {
                break;
            }
        }
        for w in &works {
            tx.execute(
                "UPDATE work_links SET work = ?1, preferred = 0 WHERE work = ?2",
                params![target, w],
            )?;
        }
        for b in &ids {
            tx.execute(
                "INSERT INTO work_links(book, work, preferred) VALUES(?1, ?2, 0)
                 ON CONFLICT(book) DO UPDATE SET work = excluded.work",
                params![id(*b)?, target],
            )?;
            tx.execute("DELETE FROM work_apart WHERE book = ?1", [id(*b)?])?;
        }
        if let Some(p) = keep_pref {
            tx.execute("UPDATE work_links SET preferred = 1 WHERE book = ?1", [p])?;
        }
        tx.commit()?;
        Ok(target as u64)
    }

    /// Takes a book out of its work, and out of the automatic grouping too, so it is not pulled
    /// back in by a matching title. A work left with one book is dissolved.
    pub fn unlink_edition(&self, book: u64) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let b = id(book)?;
        let exists: i64 = tx.query_row("SELECT COUNT(*) FROM books WHERE id = ?1", [b], |r| {
            r.get(0)
        })?;
        if exists == 0 {
            bail!("book {book} is not in the library");
        }
        tx.execute("DELETE FROM work_links WHERE book = ?1", [b])?;
        tx.execute("INSERT OR IGNORE INTO work_apart(book) VALUES(?1)", [b])?;
        prune_lone(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Lets a book take part in the automatic grouping again.
    pub fn clear_apart(&self, book: u64) -> Result<bool> {
        Ok(self
            .lock()
            .execute("DELETE FROM work_apart WHERE book = ?1", [id(book)?])?
            > 0)
    }

    /// Makes `book` the edition that opens first. `members` are the books of its group (the
    /// automatic ones too): they are linked first when they are not yet, so the choice sticks.
    pub fn prefer_edition(&self, book: u64, members: &[u64]) -> Result<u64> {
        let mut all: Vec<u64> = members.to_vec();
        if !all.contains(&book) {
            all.push(book);
        }
        let work = self.link_editions(&all)?;
        let conn = self.lock();
        conn.execute(
            "UPDATE work_links SET preferred = (book = ?1) WHERE work = ?2",
            params![id(book)?, work as i64],
        )?;
        Ok(work)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::library::Book;

    fn store_with(n: usize) -> SqliteStore {
        let s = SqliteStore::open_in_memory().unwrap();
        let books: Vec<Book> = (1..=n as u64)
            .map(|i| {
                let mut b: Book = serde_json::from_str(
                    r#"{"id":1,"file":"a.epub","meta":{"title":"T","title_sort":"T","authors":["A"],"author_sort":"A","publisher":"","language":"","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
                )
                .unwrap();
                b.id = i;
                b.file = format!("b{i}.epub");
                b
            })
            .collect();
        s.put_many(&books).unwrap();
        s
    }

    #[test]
    fn linking_makes_one_work_and_needs_two_books() {
        let s = store_with(4);
        assert!(s.link_editions(&[1]).is_err());
        assert!(s.link_editions(&[1, 99]).is_err());
        let w = s.link_editions(&[3, 2]).unwrap();
        assert_eq!(w, 2);
        let l = s.work_links();
        assert_eq!(l.len(), 2);
        assert!(l.iter().all(|x| x.work == 2 && !x.preferred));
    }

    #[test]
    fn linking_merges_existing_works_and_keeps_the_preferred_edition() {
        let s = store_with(5);
        s.link_editions(&[1, 2]).unwrap();
        s.prefer_edition(2, &[1, 2]).unwrap();
        s.link_editions(&[4, 5]).unwrap();
        let w = s.link_editions(&[2, 4]).unwrap();
        let l = s.work_links();
        assert_eq!(l.len(), 4);
        assert!(l.iter().all(|x| x.work == w));
        let pref: Vec<u64> = l.iter().filter(|x| x.preferred).map(|x| x.book).collect();
        assert_eq!(pref, vec![2]);
    }

    #[test]
    fn unlinking_remembers_the_book_and_dissolves_a_pair() {
        let s = store_with(3);
        s.link_editions(&[1, 2]).unwrap();
        s.unlink_edition(1).unwrap();
        assert!(s.work_links().is_empty(), "a work of one book is dissolved");
        assert_eq!(s.work_apart(), vec![1]);
        // linking again takes it back into the automatic grouping
        s.link_editions(&[1, 3]).unwrap();
        assert!(s.work_apart().is_empty());
        assert_eq!(s.work_links().len(), 2);
    }

    #[test]
    fn unlinking_one_of_three_keeps_the_other_two() {
        let s = store_with(3);
        s.link_editions(&[1, 2, 3]).unwrap();
        s.unlink_edition(3).unwrap();
        assert_eq!(s.work_links().len(), 2);
        assert_eq!(s.work_apart(), vec![3]);
        assert!(s.clear_apart(3).unwrap());
        assert!(!s.clear_apart(3).unwrap());
    }

    #[test]
    fn preferring_an_automatic_group_links_it_and_moves_the_flag() {
        let s = store_with(3);
        s.prefer_edition(3, &[1, 2, 3]).unwrap();
        s.prefer_edition(1, &[1, 2, 3]).unwrap();
        let pref: Vec<u64> = s
            .work_links()
            .iter()
            .filter(|x| x.preferred)
            .map(|x| x.book)
            .collect();
        assert_eq!(pref, vec![1]);
    }

    #[test]
    fn removing_a_book_drops_its_rows_and_dissolves_the_work() {
        let s = store_with(3);
        s.link_editions(&[1, 2]).unwrap();
        s.unlink_edition(3).unwrap();
        s.commit_catalog(s.next_id(), &[], &[1]).unwrap();
        assert_eq!(s.work_links().len(), 1);
        // the lone row is ignored by the app and pruned by the next change
        s.link_editions(&[2, 3]).unwrap();
        let l = s.work_links();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].work, l[1].work);
        assert_eq!(s.work_apart().len(), 0);
    }
}
