# Metadata review, series, works, preflight: notes

Internal notes for the metadata track (2026-11-12). Public docs: `wiki/docs/library.md` (Metadata, Series, Editions,
Needs attention), `wiki/docs/converting.md` (check before sending, conversion report). Pain points: PP-G-1/2/3/4/6/31,
PP-C-4/5/6.

Status: living | Last reviewed: 2026-11-23 | Owner: area agent, reviewed by @adhadse

## What was built

| Piece | Code | Tests |
|---|---|---|
| Source health + backoff | `bookx-sources/src/health.rs` (`HealthBoard`, `HealthFetcher`), wired in `metadata_fetch::fetcher()` (one global board) | 5 unit tests (states, backoff doubling, host mapping, 404 is an answer, paused source sends no request) |
| Bulk fetch logic | `src/bulk_fetch.rs` (rows, review state, `apply_rows`, `undo_applied`, `run_batch`, `MetadataPrefs`) | 9 unit tests incl. a real library apply/undo and a batch over a mock |
| Review window | `src/ui/bulkfetch.rs` | UI scenarios `meta_bulk_*`, `meta_auto`, `meta_fetch_health` |
| Provenance for more edit paths | `library_health::{record_edit_as, snapshot_meta, record_changes}`; bulk edit and tidy names in `ui/library.rs` / `ui/health.rs`, editor in `ui/editor/mod.rs`, import patches in `import.rs` + `plugins::patch_prepared_noted` | unit tests in `library_health`, `import`, `plugins`; scenarios `meta_tidy_provenance`, `patches_provenance` |
| Series | `src/series_view.rs`, window in `ui/organize.rs`, number field in `ui/metadata.rs` | 4 unit tests; scenarios `meta_series`, `meta_series_number` |
| Editions & formats | `src/works.rs` | 3 unit tests; scenario `meta_works` |
| Weak metadata | `bookx-core/src/quality.rs`, `weak:` in `query.rs`, `src/needs_attention.rs`, sidebar row | 5 + 2 unit tests; scenarios `meta_attention`, `meta_weak_filter` |
| Identifier cleanup | `bookx-sources/src/idclean.rs` | 5 unit tests |
| Preflight + report | `src/preflight.rs`, window `ui/preflight.rs`, gate in `ui/library.rs` (`preflight_gate`) | 9 unit tests; scenarios `meta_preflight`, `meta_preflight_gate`, `meta_conversion_report` |

## Decisions and caveats

* **Review before write.** `Review::new` makes rows with `accepted = false`. The only way to write without looking is the
  `auto_apply_safe` setting (default off), and it only takes rows whose current field is empty (`Row::safe`).
* **Safe = empty.** Decided by running `metadata_fetch::apply` in `FillEmpty` and `Replace` mode and comparing, so the review
  table and the single-book dialog agree.
* **Source per field.** The merged answer (`sources::Best`) does not say which source gave which field; the row shows all
  contributing sources (`openlibrary, google`). Provenance stores that string.
* **Undo.** One `update_metadata` per book = one operation-log entry; `undo_applied` calls `undo_last` N times and clears the
  provenance of the touched fields. It assumes nothing else was logged in between (the window is modal-ish, but this is
  not enforced). (Update 2026-11-13: a replaced cover is restored by undo, see the second round.)
* **`changed_fields` now counts a changed series number** as a series change (it only compared the name before). This
  also makes the single-book dialog record `series` provenance when only the number changed.
* **Health is kept between sessions** (round two); before that it was per session (in memory). `HealthFetcher` only knows Open Library, Google Books and Wikidata by host; plugin
  sources are not tracked. 404 counts as an answer; 401/403/429/5xx, timeouts and network errors count as failures. Backoff
  starts at 3 failures in a row: 30 s, doubling, capped at 15 min.
* **(Superseded in round two)** The catalogue used to be one file per book; a book can now hold several formats (see below).
* **Provenance rows need the book in the catalogue.** For imports, notes are recorded after the library was saved
  (`PATCH_NOTES` in `ui/library.rs`).
* **Patches cannot change the package file** (`patch.rs`), so a patch never changes a metadata field; the provenance records
  the field `content` with source `patch: <ids>`.
* **Preflight validates on a worker thread** (`ui/preflight.rs`) and fixes with `bookx_editing::fixes::fix_archive`; the
  original is copied to `<library>/.bookx/preflight-backups/<id>-<time>.epub` first. The fix replaces the book with
  `replace_archive`, which is not an operation-log entry (the backup is the way back).
* **Rule ids used by the fixable check** come from `FixReport::resolved_validator_ids` of a dry run on a copy.
* **Conversion report** is written by `preflight::write_converted` (also used by the dev hook). For non-EPUB outputs the result is
  not validated (the validator reads EPUB only). The font/SVG note says "may not keep them": it is a warning from the known
  differences list, not a measured fact for every writer.

## Second round (2026-11-13): formats, import lookup, authors, DRM, trackers, audiobooks

| Piece | Code | Tests |
|---|---|---|
| Several formats per book | `core::formats` (types, best format per target), `core::merge` (plan, merge, add, remove, resources), `Book.formats`, catalogue schema v6 (`book_formats` table), `bookx_store::formats` (carry collections/status/progress), `src/book_formats.rs`, `ui/formats.rs`, Editions & Formats window, Find Duplicates, details panel, book menu | `core/tests/formats_merge.rs` (11), `store/tests/formats_aliases.rs` (6), `src/book_formats.rs` (4), `src/duplicates.rs`, `src/works.rs`, CLI `tests/catalogue.rs` (3); scenarios `cat_formats`, `cat_formats_remove`, `cat_import_format*` |
| Operation log: grouped edits, covers, resources | `OpKind::{EditMany, MergeFormats, AddFormat, RemoveFormat, SetResources}`, `Edit.cover` (`CoverUndo`), `files::backup_cover` | in `formats_merge.rs` |
| Fetch at import | `MetadataPrefs::fetch_on_import` (default off), `ui/bulkfetch.rs::start_import` (no window), `ui/library.rs::finish_import`, Preferences group in `ui/metadata_settings.rs` | scenario `cat_fetch_import` |
| Response cache + persisted health | `bookx_sources::cache` (`CachingFetcher`), `HealthBoard::to_json/load_json`, `metadata_fetch::{fetcher, load_health, save_health}` | 7 + 1 unit tests; scenario `cat_cache` |
| Source per field | `sources::Best.field_sources`, `Fetched.field_sources`, `bulk_fetch::rows_for` | `sources` + `bulk_fetch` unit tests |
| Author aliases | `bookx_sources::aliases`, store table `author_aliases` (schema v6), `src/authors_view.rs`, `ui/authors.rs` | 5 + 3 unit tests, store tests, CLI tests; scenario `cat_authors` |
| DRM detection | `core::drm`, `Prepared.drm`, `Book.drm`, import refusal in `src/import.rs`, reader refusal in `app.rs`, `drm:` query | 5 unit tests + `formats_merge.rs::a_protected_epub...`; scenario `cat_drm` |
| Trackers | `bookx_sources::trackers`, `src/tracker_sync.rs`, `ui/trackers.rs` | 6 + 3 unit tests; scenario `cat_trackers` |
| Audiobooks | `bookx_sources::audiobooks`, `src/audiobook_link.rs`, `ui/audiobooks.rs` | 3 + 2 unit tests; scenario `cat_audiobooks` |

### Decisions and caveats (round two)

* **Files stay put.** Merging does not rearrange the library: the file of a merged book moves into the folder of the book that
  stays (when that has a folder of its own), the rest of its old folder (cover, sidecars) goes to the trash with its record. In a
  *flat* or *linked* library the files stay where they are. The primary `file` is always the best format for the reader (EPUB),
  also when the book that stays had only an AZW3.
* **Two files of the same format are not formats.** `plan_merge` refuses them with the reason; that is Find Duplicates' job.
* **What a merge does not carry over:** highlights, bookmarks and the reading position of the merged copies (they refer to another
  file's spine); they stay in the trash record and come back with Undo. Collections, status and the furthest progress are carried
  (`bookx_store::formats::carry_state`) and are not taken back by Undo (harmless extras on the kept book).
* **Everything non-EPUB is carried, not read.** The reader, editor, metadata write-back and conversion work on the main EPUB. AZW3 etc.
  are sent, opened with the system's default application and listed.
* **Imports are still converted to EPUB.** *Add as another format* keeps the incoming file *as it came* (for example `.azw3`),
  next to the book. The offer is made when the same identifier or title+author is in the library and the incoming extension is a
  format the book lacks; an incoming EPUB of a book that has an EPUB is still just "already in library".
* **Format rows in SQLite are an index.** The truth is the book's JSON (`Book.formats`); `book_formats` is rewritten on every
  `put_row` and backfilled by the v6 migration. Backups carry it, older backups are migrated on restore (tested with a v5 file).
* **Cover undo** keeps the old cover under `.bookx/cover-backups` (8 MB each, 64 MB in all, oldest first; names start with the time).
  Only covers replaced through `update_metadata` are logged (the metadata dialog, bulk fetch). The editor's `replace_archive` is not
  an operation-log entry, so its cover changes are not restorable (same as before). Undoing the *first* cover a book ever got
  removes the cover from the manifest, but a file called `cover*` in the manifest is guessed as the cover again by `Package::cover_item`,
  so the catalogue may keep showing it (an existing quirk of `remove_cover`).
* **Cache and terms.** TTLs: Google Books 6 h, Open Library and Wikidata 24 h, "not found" 1 h, everything capped at 24 h and 32 MB; only
  status 200 bodies and 404s are stored; no personal data is in the URLs (title/author/ISBN). The terms of the sources were not read
  clause by clause for this change; the TTLs are deliberately short. The cache sits outside the health fetcher, so a cached answer
  neither counts for nor is blocked by the health board. Plugin sources are not cached. UI tests with canned answers skip the cache
  unless `BOOKX_SOURCES_CACHE=1`.
* **Health file:** `<cache dir>/sources/source-health.json`; entries older than a week are ignored; a running pause continues, minus the time that passed.
* **Fetch at import** runs through the same `bulk_fetch` code as the window (never presented), with `auto_apply_safe` forced on. It
  writes empty fields only and records provenance. Proposals that would replace a value stay unapplied and are mentioned in a toast.
* **Aliases** point from a spelling to one canonical name; setting an alias re-points aliases that pointed at the alias (no chains).
  Matching is by `name_key` (case, accents, `Last, First` order). Transliterations are declared by hand.
* **DRM** is recognised by markers only: `META-INF/encryption.xml` algorithms other than font obfuscation, `rights.xml` with the ADEPT
  namespace, `license.lcpl`, `sinf.xml`, the MOBI encryption type, the `DRMION` header, Adobe PubSec in PDFs. A KEPUB (by file name) with
  encrypted content is called Kobo; other encrypted content is "encrypted". Never tested with real purchased files, only synthetic fixtures.
* **Trackers:** columns are found by name (Goodreads and StoryGraph layouts verified from their documented exports, Hardcover by name only).
  Ratings use 0 to 5 stars (the library keeps 0 to 10). "Read" sets the reading status to Finished and the last-read date; "to read" adds a
  `to-read` tag; shelves become tags. Reading status and last-read date live in the catalogue only, so Undo takes back ratings and tags.
* **Audiobooks:** a folder with two or more audio files (or `.m4b` files) is one audiobook; a lone `.m4b` is one. Pairings under 0.8 are
  dropped. Nothing is played. Resources are absolute paths and are not moved with the library.

## Not done

* The check before sending in the Device window (`src/devices.rs` / `ui/devices.rs` belong to another track).
* Language-aware name order (the alias table is built; the name-order toggle is not).
* Live-network tests of the sources; a clause-by-clause terms review for the cache TTLs; caching for plugin sources.
* Hardcover API sync; checking the Hardcover CSV layout against a real export.
* Real DRM files (only synthetic fixtures), LCP support, any form of DRM removal (by design).
* Editions as a level above books (a work id); progress shared between an EPUB and an audiobook; playback.
* Screenshots of the new windows (Editions & Formats merge, Authors, Reading Trackers, Audiobooks) are not on the wiki.
* Mockups (`cargo run --example mockups`) were not updated; the new windows have no screenshots on the wiki.
* Editor metadata tab: provenance is recorded when the form is applied, not when the book is saved; no UI scenario for it
  (the logic is the same `record_edit_as` that is unit tested).
* Bulk edit provenance is covered by a unit test (`library_health`), not by a UI scenario: the dialog has no dev hook.
  (Update 2026-11-13: `library_health` unit tests extended; the editor tab still has no scenario.)
* No real-device test of any Kindle/Kobo rule; the Amazon rules are the ones listed in PP-G-6 (language, DRM, size, cover, author).
