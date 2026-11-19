# Your data and leaving Bookx

Your books are ordinary files and the catalogue is an ordinary SQLite database, so nothing is locked in. This page says
where everything is, so another tool (or you, with a script) can read it, and how to take it all with you.

## What is in a library folder

| Path | What it is |
|---|---|
| `<author>/<title> (id)/…` (or the layout you chose) | The book files (EPUB and the other formats you added) and their `cover.jpg`. They are normal files: open them with any reader. Metadata edits are written **into the EPUB** as well as the catalogue. |
| `bookx-library.db` | The catalogue (SQLite, WAL mode). See below. |
| `.bookx-backups/` | Snapshots of the catalogue (SQLite files), see [Backup and restore](backup-and-restore.md). |
| `.bookx-trash/` | Deleted books waiting for *Empty Trash*: the files and an `entry.json` with the catalogue record. |
| `.bookx/` | Small JSON state files: `oplog.json` (the undo log), `layout.json` (where books are filed), `inbox.json` (the watched import folder), `fixity.json` (file checksums), `not-duplicates.json` (pairs you said are different books), `cover-backups/` (covers replaced by a lookup). Safe to delete; you lose the undo history and these memories only. |
| `.bookx-lock` | Which Bookx has the library open ([read-only mode](library.md#locking-and-read-only)). |

## The catalogue (`bookx-library.db`, schema version 6)

Open it with any SQLite tool, **after closing Bookx or on a copy**. Do not write to it while Bookx runs. The schema
version is in `PRAGMA user_version`; a newer database than the app knows is refused rather than damaged.

| Table | Columns | Meaning |
|---|---|---|
| `books` | `id`, `title`, `title_sort`, `author_sort`, `series`, `series_index`, `added`, `modified`, `last_read`, `progress`, `data` | One row per book. `data` is the whole record as JSON (authors, publisher, identifiers, tags, description, rating, file paths, bookmarks, highlights, format files, resources). Times are Unix seconds. |
| `book_formats` | `book`, `file`, `format`, `size`, `main` | Every format file of a book; `main = 1` is the primary one. |
| `book_status` | `book`, `status` | `new`, `reading`, `on_hold` or `finished`. |
| `collections`, `collection_books` | | Shelves and which books are on them. |
| `provenance` | `book`, `field`, `source`, `at` | Where each field came from (a metadata source, the file, *you*). |
| `author_aliases` | `alias`, `canonical` | Spellings and pen names of authors. |
| `text_stamps`, `fts_meta`, `fts_text` | | The search index. Rebuilt by Bookx when missing; ignore it. |
| `meta` | `key`, `value` | Counters (`next_id`). |

## Getting everything out

- **`bookx-cli library dump`** prints the whole catalogue (every field of every book) as JSON.
- **`bookx-cli library cite`** writes a bibliography ([BibTeX, RIS, CSL JSON, APA](library.md#export-citations)).
- **Export Annotations** (main menu) writes your highlights and notes for the library as Markdown, HTML, JSON, text,
  Kindle clippings or a Readwise file; see [Highlights and notes](annotations.md).
- **Reading Trackers** exports your status, rating and dates in the Goodreads and StoryGraph layouts
  ([library](library.md#reading-trackers)).
- **The files themselves** are in the library folder (or, with *Link in place*, where they always were).

Bookx has no export to Calibre's `metadata.db` yet: you can import from Calibre, not back.
