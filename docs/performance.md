# Performance at scale (PP-C-11, PP-G-12)

Internal notes: what was measured on large synthetic libraries, what was slow and fixed, and what is left. The
budget tests themselves are described in `docs/perf.md`.

## How it is measured

- **Synthetic libraries**: `crates/bookx-store/tests/common/synth.rs` builds book *N* of a library of *M* books as a pure
  function (syllable-made titles and names, about 8 books per author with a skewed spread, a quarter of the books in a
  series, 400 tags, a few with a second format, every 500th book a near-duplicate of the one before). No real content,
  no files. The same generator is included by `bookx-core`, `bookx-sources` and the example through `#[path]`.
- **`make scale`** (`cargo run --release -p bookx-store --example scale -- 10000 50000 100000`) builds the catalogue on
  disk and times every step, with RSS and file sizes. "Cold" means a fresh process opening the file; the OS page cache
  is warm (dropping it needs root).
- **Budget tests** (run by `make perf`, scaled with `BOOKX_PERF_SCALE`): `crates/bookx-store/tests/perf_budgets.rs`
  (`a_hundred_thousand_book_library`: save, reopen, RSS growth, sorts, filters, categories, FTS, backup, snapshot,
  integrity), `crates/bookx-core/tests/perf_budgets.rs` (`books_for_ids_keeps_order_and_is_linear`),
  `crates/bookx-sources/tests/perf_budgets.rs` (`duplicate_scan_of_a_hundred_thousand_books`).
- **Broadway scenario** `scale_grid_20k` (`build-aux/ui-tests-scale.sh`): 20,002 books through the Calibre importer,
  then `library:requery-time` (rebuild of the grid and table models) and `library:grid-bench=90` (scrolls the cover
  grid by a third of a screen on every frame and reports the frame intervals).

Machine: AMD Ryzen 9 9950X (32 threads), 64 GB, NVMe, Linux, rustc 1.93, release builds, 2026-11-20. A slower machine
is several times slower; the budgets have room for that.

## What was slow, and the fix

| Hot path (100,000 books) | Before | After | Fix |
|---|---|---|---|
| Duplicate scan (`dedup::find_duplicates`) | **22.0 s** | **about 1.2 s** | Pairs inside a block (same author surname) were scored with the full title normalisation, Levenshtein and Jaro-Winkler. A pair is now skipped when a bound from the shared characters (length and character histogram) proves its title score is below the threshold; ISBN matches and titles with a subtitle are never skipped. Tested against pairwise scoring. |
| Library grid: books for the visible ids (`requery`) | **3.1 s** (O(n²): a `Library::get` per id; 0.75 s at 50k, 23 ms at 10k) | **2.5 ms** | `Library::books_for(ids)`: one pass with a map. |
| Save after one change (every page turn) | **108 ms** | **19 ms** | `Library::save` hashed every book by serialising it to a `String`; it now serialises into a reused buffer on up to 8 threads for libraries of 4,096 books or more. |
| Open the catalogue | 305 ms | 205 ms | the same hashing at open |
| List sorted by title / author | 82 / 128 ms | 26 / 29 ms | the sort key was lower-cased in every comparison; it is computed once per book |

| **First frame of a 20,000 book cover grid** (real GNOME session, debug build) | **14.5 to 26 s** (one frame) | **0.6 s**, then 62 ms per frame on average while scrolling a screenful per frame | `bind` called `set_tooltip_text` on every tile; GTK answers that by picking the whole window at the pointer, which recomputes the style of every widget, for each of the tiles bound while the model is laid out (stack found with gdb: `gtk_widget_set_tooltip_text` → `gtk_widget_trigger_tooltip_query` → `gtk_widget_do_pick` → CSS lookup). The title label now answers `query-tooltip` on demand. |

| **Sort / search / clear search of a 2000 book grid** (real GNOME session, release build, longest block of the main loop) | **997 / 448 / 782 ms**, back to the first sort 797 ms (QA round 2 saw 930 / 480 / 810) | **24 / 23 / 21 ms**, back 19 ms | Two causes. (1) `set_max_columns(40)`: GtkGridView keeps `max_columns * 32 + 1` tiles alive around its anchor (`gtk_list_base_set_anchor_max_widgets`), 1281 bound tiles for a screen of 20; the cap now follows the width and the cover size (`src/grid_columns.rs`: 385 tiles at first, 8 columns = 257 once the window is laid out; the grid is never capped below what fits). (2) The model was filled in one call; `src/grid_fill.rs` hands the books out in pieces of about 8 ms (the first piece, 48 books or enough to cover the scroll position, in the same main-loop turn so the grid is never empty), the rest in idle steps below the redraw priority, cancelled by the next requery. The table is filled only while it is shown. |
| Open Notes & Highlights (2000 books, release, real session) | 1030 ms | no stall of 100 ms or more | it was the rebuild of the grid behind the view switch, not the notes data (`notes_view::rows` is an in-memory pass over the highlights; the window already lists 200 at a time). Nothing was cached. |

Nothing else needed a change at 100k.

## Numbers (release, after the fixes; before in brackets where it changed)

| Step (ms) | 10,000 | 50,000 | 100,000 |
|---|---|---|---|
| First save of a new library (bulk insert, one transaction) | 681 | 3,927 | 8,459 |
| Open the catalogue (load all books into memory) | 23 (32) | 110 (147) | 205 (305) |
| List, sort by title | 2.2 (5.2) | 12.6 (32) | 25.7 (82) |
| List, sort by author | 2.4 (8.5) | 14.0 (54) | 28.9 (128) |
| List, sort by date added | 1.2 | 7.9 | 18.8 |
| List, sort by series | 13.4 (18) | 78 (113) | 165 (233) |
| Filter: series, tag, rating, free text, language+tag | 0.1 to 2.0 | 1.3 to 14 | 2.4 to 26 |
| Sidebar categories (authors, series, tags) | 3.1 | 22.7 | 46.9 |
| Grid model: books for the visible ids | 0.2 (23) | 1.2 (746) | 2.5 (3,147) |
| Store: ids sorted by title (SQL) | 0.4 | 2.4 | 4.2 |
| FTS metadata search, rare word | 0.3 | 0.8 | 1.2 |
| FTS metadata search, word in half the books | 25 | 128 | 252 |
| Content text index of 2,000 books / search | 39 / 0.1 | 40 / 0.1 | 40 / 0.1 |
| Save after one change (page turn) | 2.5 (11) | 10.5 (52) | 19.1 (108) |
| Save with nothing changed | 2.0 (11) | 8.4 (55) | 18.7 (115) |
| Online backup of the catalogue | 9.5 | 42 | 77 |
| Integrity check | 134 | 562 | 1,205 |
| Duplicate scan (`bookx-sources`) | not measured | not measured | 22,000 before, about 1,200 after |

| Size | Catalogue file | Backup | Memory (RSS) after open |
|---|---|---|---|
| 10,000 books | 29 MB | 29 MB | 27 MB |
| 50,000 books | 121 MB | 121 MB | 93 MB |
| 100,000 books | 235 MB | 235 MB | 182 MB |

About 2.3 kB of catalogue per book on disk (JSON record plus two full-text indexes) and 1.8 kB in memory.

## The window with 20,000 books

- Broadway (`scale_grid_20k` in `build-aux/ui-tests-scale.sh`, debug build): rebuilding the grid and table models of
  20,002 books takes about 350 to 640 ms (the pure part is the 2.5 ms above; the rest is creating the GTK list items,
  linear in the number of books). Broadway has no frame clock without a browser, so scrolling cannot be timed there.
- Real GNOME session (`library_scroll_20k` in `build-aux/session/scenarios_scale.py`, real Mutter, debug build of
  the app, one session at a time): a frame callback scrolls the cover grid by a third of a screen on every frame for
  120 frames (about 100,000 px) and reports the frame intervals: average 62 ms, worst 600 ms (the first frame, which
  lays out the model); then 40 real wheel clicks (the harness sends one every 50 ms), after which the accessibility
  tree answers in 0.15 s. Before the tooltip fix the first frame took 14.5 to 26 s and the scenario fails its 3 s limit.
  A release build is several times faster per frame; no release figure was taken. This measures the frame clock's
  interval, not the pointer-to-photon latency (that needs a camera or a compositor trace).
- The grid does not create widgets only for the visible tiles: GtkGridView keeps `max_columns * 32 + 1` bound tiles alive
  (see the table above), which is why the column cap follows the window width. Scrolling does not depend on the library
  size apart from the model rebuild and the first layout.
- Stalls of the grid (`library_stalls_2000`, real session; `grid_stalls_2000`, Broadway): `library:stalls` sorts, searches
  and clears the search while a 4 ms timer measures the longest gap; `library:fillstate` prints the tiles alive, the
  scroll position (`adj=value/page/upper`) and whether pieces are still to come; `BOOKX_FILL_SYNC=1` fills in one call
  like before (to measure against). Broadway debug build: 270 to 290 ms before, 80 to 100 ms after (Broadway frames
  are not representative; the real-session numbers above are).
- Not done: a spinner. A fill that runs longer than 200 ms only changes the pointer to the busy cursor over the list
  (nothing to animate on e-ink); in practice the list is complete within about half a second at 2000 books.

## Limits and not done

- **First save of a large library is slow-ish**: 8.5 s for 100,000 books, almost all of it writing two FTS5 indexes
  (trigram for metadata). It happens once (import of a Calibre library of that size, or the JSON migration). Importing
  100,000 books through the importer runs in one transaction, so it is not worse than that.
- **The whole catalogue is kept in memory** (`Library.books`): 182 MB at 100,000 books. A 1,000,000 book library
  would need about 1.8 GB; not a target. Opening takes about 200 ms at 100,000 because every record is parsed.
- **`SqliteStore::search_ids` for a very common word** (250 ms at 100,000) loads every candidate record. The app does
  not use it (the library view filters in memory); only API users do. Not changed.
- **Cold start of the whole app** (GTK, plugins, fonts) was not measured on a 100,000 book library; only the catalogue
  part (open 205 ms), the model rebuild and, at 20,000 books in a real session, the first frame (0.6 s, debug) are known.
- **Grouped editions (`work_groups::group`)** and the table view's model were not measured at 100,000 books.
- **Selecting all** of 100,000 books runs per-book `Library::get` lookups in a few actions (selection sized, not
  library sized); a Select All followed by a bulk action on 100,000 books has not been profiled.
- The page cache was warm for all "open" numbers; a real cold start from a spinning disk reads the 235 MB file once.
- Broadway cannot time frames at all (no frame clock without a browser). The real-session scroll test uses 20,000
  books, not 100,000, in a debug build, with a virtual monitor (software GL), so it catches multi-second stalls, not
  small differences.

## Big books in the reader and the editor (wave 6, QA round 2)

Book: `build-aux/make-big-epub.py OUT 20000`, 10.3 MB, 20 chapters of one 450 KB line each, a nested navigation
document with 20 020 entries (1.2 MB, one line). Release build, Broadway, `BOOKX_STALL_WATCH=60` (longest
stretches in which the main loop did not run, per command; the first number of a command is the work, the others are
frames after it).

| Action | Before | After | What changed |
|---|---|---|---|
| open in the reader | 1133 ms | about 600 ms | the Knuth-Plass planner asked Pango for `index_to_pos` of every word edge (each call walks the paragraph, quadratic); one cluster pass now builds the table (`canvas_xtable`). What is left is shaping 1500 paragraphs of the first chapter at once |
| open in the editor, all stalls added | 7.5 s (4.0 s Contents list, 2.0 s GtkSourceView, 1.2 s other) | about 2.2 s | the Contents list is a virtual `ListView` (20 020 rows were 20 020 widgets); a file with a line over 50 KB gets no syntax colouring (GtkSourceView's highlighter is quadratic in the line length and gave up after 2 s); the file list read the navigation document once per file (20 times), now once |
| open a chapter that is one 450 KB line | 2.4 s (debug) + repeated 150-300 ms | page with "Add Line Breaks and Open" at once | GtkTextView lays a line out as one Pango paragraph; the file is no longer put in the view until the user chooses |
| open the 1.2 MB navigation document | 25 s (hints applied 20 000 link tags, each one splits the line again) + 7 s of text view layout | 0.6 s, then the same page | no hints and no spelling for a file with one huge line; text view not used |

Not done: the 0.6-0.9 s of laying out the first chapter (reader and editor preview) stays on the main thread. A chapter is
laid out as a whole because page numbers, anchors and the saved position depend on the complete layout; laying out in
slices with a progress indicator needs the position restore, selection and page count to learn to cope with a
chapter that is still growing. The numbers above are for chapters of 1500 paragraphs (450 KB); a book with the same
10 MB in 200 chapters lays out each chapter in a few milliseconds.
