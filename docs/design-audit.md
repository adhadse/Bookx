# Design audit: how Bookx stores its data (2026-11-23)

Internal. Prompted by the owner's question: "highlights, notes and bookmarks are not rows of their own, every book is one
JSON blob in a `data` column; that must be slow". Measured, not guessed, and the other places with the same shape listed.

## 1. What is stored where

| Data | Where | Shape |
|---|---|---|
| Books, their metadata, **highlights, notes, bookmarks**, reading position | `<library>/bookx-library.db`, table `books`, column `data` | one JSON document per book (sort keys, progress, dates are real columns beside it) |
| Search over metadata and book text | FTS5 tables in the same database; the full text of books in a separate `~/.config/bookx/index/fulltext.db` (22 MB here) | rows |
| Collections, status, custom columns, works, item notes | same database, real tables (schema v3..v9) | rows |
| Library operation log (undo), trash | `<library>/.bookx/oplog.json` (200 ops), `.bookx-trash/*/entry.json` | JSON files |
| Reading statistics (every session), vocabulary, KOReader annotation ledger and raw received annotations, KOSync state, sync activity, shelves, news, dwell, editor state, settings | about 16 JSON files in `~/.config/bookx/` | each rewritten whole |

## 2. Measurement: annotations as JSON inside the book row

Synthetic libraries, release build, this machine (`crates/bookx-store/tests/annot_scale.rs`, run by hand):

| | 2 000 books x 40 highlights (80 k) | 10 000 books x 60 highlights (600 k) |
|---|---|---|
| database size | 25 MB | 196 MB |
| open the library (load and parse every book) | 80 ms, +22 MB memory | 266 ms, +104 MB memory |
| save after adding ONE highlight | 35 ms | 33 ms |
| scan every highlight in memory (Notes view) | 0.7 ms | 5 ms |
| the same question in SQL over the JSON column | 55 ms | 401 ms |
| (comparison) a real `highlights` table: same LIKE question | 5 ms | 34 ms |
| (comparison) edit one highlight in a real table | 25 us | 31 us |
| (comparison) all highlights of one book, indexed | 30 us | 36 us |
| (comparison) table size on top of the JSON | +19 MB | +144 MB |

Reading of the numbers:

* **At realistic sizes the JSON-per-book design is not slow for the app.** The whole library lives in memory (`Library`
  loads every book once, `CatalogBackend::load`), the Notes view and every filter scan memory, and a save rewrites only the
  books that changed (a 20 KB row, about 33 ms, mostly the commit). Opening 600 000 highlights takes a quarter of a second.
* **It is the wrong shape for what we now want:** a row per annotation is about 1 000x cheaper for a single edit (31 us
  against 33 ms) and about 12x faster for a query that has to go through SQL (34 ms against 401 ms). More important than the
  speed, a row has an **identity**: the history/restore feature, the sync ledger (which device saw which highlight), per-
  annotation conflicts, full-text search over notes and queries from the CLI, the MCP server and the sharing server all want
  to say "this highlight", and today the only unit they can name is "this whole book".
* **Costs that grow with the size of the annotations, not the number of books:** start-up time and memory (linear, 104 MB at
  600 k highlights), and the sharing server (`CatalogWatcher`) which re-reads and clones the whole catalogue on every
  commit while a reader saves progress.

Verdict: not an emergency, a ceiling. Do it as a planned schema change (below), not as a rushed one.

## 3. Other gaps of the same kind (flagged, ranked)

1. **No safety net around a schema update (fixed in this change).** `migrate()` ran each step in a transaction, but there
   was no copy of the old database, no check afterwards, no answer for a full disk, and an older Bookx refuses a newer
   database with no way back. Now: copy first, refuse to start when the copy cannot be made, integrity and book-count check,
   newest two copies kept (`bookx-library.db.pre-v<N>-<time>`).
2. **State sprawl: about 16 JSON files in the settings folder, none with a schema version.** `stats.json` holds every reading
   session ever and is rewritten whole at the end of each session; `annotation-sync.json` (the ledger, 356 KB here) and
   `koreader-annotations.json` (every annotation a device sent, 283 KB) grow with use and are rewritten whole; settings use
   ad-hoc "reset once" flags instead of a version. Consequences: unbounded growth, whole-file rewrite on every change, no
   transaction across files, nothing migrates them in a controlled way. Reading statistics and the sync ledgers describe the
   library, yet live in the user's settings folder, so a library backup or a copy of the folder to another computer does not
   carry them. Recommendation: move statistics, vocabulary, ledgers and the annotation journal into the library database
   (tables, schema-versioned, inside the backup), keep only preferences as JSON, and give the settings file a `version`.
3. **The in-memory library is the only query path.** The CLI, the MCP server and the sharing server cannot ask a question
   about annotations or tags without loading and parsing everything (`all_books`). Tags, authors and identifiers are inside
   the JSON too, so SQL cannot filter by them (only the sort keys and FTS are real). Fine for the app today; the ceiling
   for headless tools. Needs the annotation tables of section 4 and lazy per-book loading.
4. **The operation log is a JSON file next to the database, not part of it.** No atomicity with the change it describes
   (a crash can leave a change without its undo entry or the reverse), capped at 200 entries. The annotation history must
   not copy this design: it goes into the database and is written in the same transaction as the change.
5. **Two databases for search.** Book text lives in `index/fulltext.db` outside the library; a library copied to another
   computer has to re-index (the stamps table exists for this). Acceptable (it is a cache), but say so in the backup docs.
6. **Single writer by lock file, last writer wins per book.** With the whole book as the unit of write, two processes
   editing different highlights of one book would overwrite each other if the lock were ever bypassed. The lock holds today.
   Rows per annotation remove this class of bug.

## 4. Plan (phased, every step migrates by itself)

Rule for every schema change from now on (also in AGENTS.md section 5):

* a numbered migration that runs in one transaction on open, with no user action;
* the pre-update copy and the check after it (done here: `bookx-store` `SqliteStore::open`);
* a test that opens a database turned back to the previous version and one built by the previous release's code, and
  proves nothing was lost (counts of books and annotations equal before and after);
* an older Bookx keeps refusing a newer database with a clear message, and the copy is the way back;
* for a big library a progress dialog ("Updating your library") instead of a frozen start-up (the migration of 600 k
  highlights is a few seconds; measure it in the migration test and budget it).

Steps:

1. *(done in this change)* update safety net.
2. *(in progress, other branch)* `annotation_history` table written by the store in the same transaction as the change.
3. `highlights` and `bookmarks` tables (v11): migrate every book's JSON arrays into rows inside the migration transaction,
   verify per book and in total that the counts are equal, then drop the arrays from `data` (keep them one release behind
   a flag if you want a longer safety net). `Book` keeps its fields; the store joins them on load. Lazy loading
   (annotations of the open book only; the Notes view asks the table) follows when the memory number matters.
4. Statistics, vocabulary and the sync ledgers into the database (v12+), importing the old JSON files once and keeping them
   as `.migrated` like the old catalogue was.
5. Settings file gets a `version` and per-version steps.

Open decision for the owner: how far to go before 1.0. Step 3 is the one with real value (history, ledger, headless
queries); step 4 fixes the backup gap.
