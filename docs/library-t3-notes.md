# Library and store, track T3 (wave 11): notes

Internal. What was built, how it is tested, caveats and what is not done. Public documentation is in
`wiki/docs/library.md` and `wiki/docs/command-line.md`.

## Built

| Item | Where | Tests |
|---|---|---|
| Custom columns (L3/L25) | `bookx-core::custom` (types, `normalize`, `compare`, `Library::set_columns/edit_custom/set_custom/sort_by_custom`, composite renderer registered by the programs), `bookx-store::custom` (schema 8, table `custom_columns`), values in `Book::custom` (so in every snapshot, JSON dump and backup), `bookx-sources::template::render_composite` (`{#label}` fields) | `custom.rs` unit tests, `core/tests/custom_columns.rs`, `store/tests/custom_columns.rs` (persistence, snapshot restore, v7 upgrade, JSON round trip), `calibre_export.rs` (round trip, `calibredb list` opens it), CLI test, perf budget over 100 000 books, scenario `lib12_custom_columns` |
| Query language (L22) | `bookx-core::query`: AST with parentheses, `AND`/`NOT`, `~` regex, date fields and relative dates, `#label`, `marked`, `id` | 11 unit tests incl. hostile nesting and ReDoS-style patterns |
| Search and replace (L16) | `bookx-core::replace`, `ui/replace_fields.rs`, `library replace` | unit + `core/tests/custom_columns.rs`, CLI test, scenario `lib12_records_replace_marks` |
| Empty record / ISBN (L14) | `bookx-core::empty`, `src/library_add.rs`, `ui/import_isbn.rs` | unit, scenario |
| ZIP with several books (L14) | `bookx-core::archive_books`, `ui/library_t3.rs` (`split_bundles`, `pick_bundle`) | 4 unit tests (zip-slip names, bomb ratio, folder entry); the picker dialog itself is not driven by a scenario |
| File name guess (L15) | `src/library_add.rs::apply_guess`, called in the import handler of `ui/library.rs`; Preferences rows in `ui/metadata_settings.rs`; config `guess_filename` (default on), `filename_patterns` | unit (5) |
| Marks (L21), quickview (L19) | `Book::marked` (serde skip), `src/library_related.rs`, `ui/library_t3.rs` | unit, scenarios |
| Notes on authors/series (L20) | `bookx-store::notes` (schema 9, table `item_notes`) | unit (3), scenario `lib12_quickview_notes` |
| Tag tree (L18) | `src/library_tagtree.rs`, switch in `ui/library_sidebar.rs`, rows in `category_row` | unit (2), scenario `lib12_tag_tree` |
| Send shelf (L5) | `ui/shelves.rs::send_shelf` (sets the Devices selection, opens the window) | scenario `lib12_shelf_send` |
| Folder watcher (L12) | `bookx-core::inbox` (`watch`, `pending_files`, `mark_done`), `ui/folder_watch.rs`, switch in Library Health | unit (4), scenario `lib12_inbox_watch_and_index` |
| Text index status and rebuild (L33) | `SqliteStore::clear_text_index`, `ui/index_status.rs` | store test, unit, scenario |
| Jobs (L27) | Devices window sends and Add from the Web are jobs | scenario `device_ui_job` |
| Catalogue EPUB (L28) | `bookx-core::catalog_epub`, `catalog --format epub`, `#label` columns | unit (2), CLI test |
| Cover from PDF, trim cover (L13) | `src/library_cover.rs`, Edit menu | unit (2) |

## Design notes and caveats

- A custom value is stored as a normalised string in the book record. Ratings are stars (0.5 steps), not the 0-10 of
  `meta.rating`, so the untyped matcher in `query.rs` can compare every numeric column the same way. Composite values
  are stored too and recomputed by `Library::refresh_composites_of` after an import, a metadata update and an edit of
  custom values; a program that never registers the renderer (`register_composite_renderer`) gets no composites.
  `bookx-cli`, the app and `library_health::open` register it; the MCP server and `bookx-serve` do not, they only read.
- `Library::edit_custom` and `set_columns` log `OpKind::SetCustom` (undo of values). Removing a column is not undoable.
- Marks live in `Book::marked`, which is `#[serde(skip)]`; a reload of the library from the database forgets them (the
  intent: temporary). `book_hashes` ignores them, so marking never causes a save.
- Adding fields to `Book` meant adding `custom` and `marked` to every `Book { .. }` literal (about 30 places in other
  crates' tests and code); a future field needs the same, or `Book` should get `Default`.
- The saved-search expander (`src/saved_searches.rs`) still splits on `OR` words without knowing about parentheses: a
  user query with parentheses and a `search:` term whose saved query has `OR` can group differently than written.
- The table's custom columns are plain `gtk::ColumnViewColumn`s added next to the typed ones (`ui/custom_columns.rs`,
  `sync_table`). Their header sorter is remembered in `Config::custom_sort` together with the built-in sort that was
  set at that time; choosing another sort in View options replaces it.
- The Calibre export writes custom columns in Calibre's table layout (normalized tables carry the `link` column Calibre
  reads); the per-column fkc triggers Calibre creates for itself are not written. Calibre's own template (`composite`)
  columns are not imported.
- `calibredb list --fields "*pages"` on an export was run by hand and by `custom_columns_go_to_calibre_and_come_back`
  when `calibredb` is installed; the test passes without it.

## Security checklist (docs/security.md section 8)

- New parser of untrusted bytes: `archive_books` (ZIP). Names are display only, written under a sanitised base name in a
  folder the app chose; 20 000 entries, 1 GiB per entry, ratio 200 over 10 MB, a limited copy loop that stops even when
  the header lies, encrypted entries refused. Tests: `extraction_never_leaves_the_folder_and_never_overwrites`,
  `a_compression_bomb_and_a_folder_entry_are_refused`.
- User-supplied regular expressions (search and replace, `~` in queries, file name patterns): the `regex` crate (linear
  time), size limit 1 MiB, pattern length 256 (query) or 1000 (replace), invalid patterns match nothing / are errors.
  Parentheses and `NOT` chains are depth-limited (tests `parentheses_group_and_negate`).
- Custom values: 10 000 characters, 64 columns, enumeration 200 values; labels are `[a-z0-9_]`, so they cannot reach SQL
  or paths (the values are stored through bound parameters, the Calibre export builds table names from numbers only).
- No new process spawns, listeners or network access. The ISBN record path reuses the existing Fetch Metadata flow
  (`Fetcher` with the SSRF guard). The folder watcher reads a folder the user chose and imports through the normal path.
- No new dependency.

## Not done (and why)

- L17 bookshelf and cover-flow views: a new view family in the grid; not started.
- L14 RAR/7z archives and "add data files to a record": no RAR reader in the tree (T4 owns real RAR); data files need a
  design for where they live.
- L13 interactive crop: only the centre trim to 2:3 and the PDF first page exist; a drag-to-crop dialog is not built.
- L27 device cover sync as a job.
- L18 drag and drop assignment of tags; L20 notes for tags and publishers have a store API but no editor, notes are not
  shown in the details panel; L28 sections, prefixes and exclusions of the catalogue.
- Custom column rules and colouring (L25).
- The MCP server does not expose custom columns yet (its `Book` serialisation is its own).
- No scenario drives the ZIP picker dialog, the Add by ISBN dialog (the dev hook adds the record directly) or the
  Custom Columns dialog; they compile and share their logic with tested code. A session test (`build-aux/session-tests.sh`)
  was not run.

## New message ids

Listed in the change description; `po/` is untouched.
