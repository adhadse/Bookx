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
- **Cold start of the whole app** at 100,000 books is measured since X18 (below); a real session (GNOME Shell) was not.
- **Grouped editions (`work_groups::group`)** and the table view's model were not measured at 100,000 books.
- **Selecting all** of 100,000 books: profiled and fixed in X18 (below). A bulk action on the selection (edit metadata of
  100,000 books at once) was not profiled.
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

The 0.6-0.9 s of laying out the first chapter was left on the main thread at that time; it is gone since X18 (next
section). The numbers above are for chapters of 1500 paragraphs (450 KB); a book with the same 10 MB in 200 chapters
lays out each chapter in a few milliseconds.

## X18: large books and large libraries (2026-11-22)

Same machine and rules as above (Ryzen 9 9950X, release build, Broadway display, `BOOKX_STALL_WATCH` = longest stretch
in which the main loop did not run). Every "before" was measured on the build just before the change, with the old
behaviour kept behind an environment variable where one exists (`BOOKX_LAYOUT_SYNC=1`).

### What was done

- **Chapters are laid out in slices** (`canvas.rs`: `flow_slice`, `FlowCursor`, `Job`). `build_flow` is now a loop that
  can stop between two blocks and carry on later; a chapter of 120 blocks or more (`flow_window::SLICE_FROM`) runs in
  slices of 8 ms (`SLICE_MS`) on an idle source below the redraw priority, so input and drawing go first. The page is
  shown as soon as the place it was opened at is laid out (`Ready`: the first two screens, an anchor and a screen after
  it, a character and a screen after it, or the whole chapter for "last page"), then the rest follows. While it is
  incomplete `pos_info` reports an estimate of the page count (from the characters laid out so far) and the fraction of
  the estimated height; the exact numbers replace them when the chapter is complete, and the position callback is fired
  again then (the reader marks vocabulary and refreshes the page count at that report).
- **Windowed layout** (`src/flow_window.rs`, pure, unit tested): a saved position beyond 3 % of a chapter of 240 blocks or
  more is not reached by laying out everything before it. The height of every block is estimated from its characters and
  the measured font (one wrapped sample paragraph), corrected by laying out 16 blocks spread over the chapter for real
  (`calibrate`), the block where the target should be (a screen before it) is found by `window_start`, and a window of
  about three screens is laid out from there and shown (`y_off` is how far down the chapter the window starts). The
  whole chapter is then laid out from its start beside the window (`Job::buf`, swapped in and out of `State` per slice).
  When it is complete it replaces the window: the exact fraction if the page was not touched, otherwise the very same line
  (found through the block starts of both layouts). Not used for vertical text, fixed layout and chapters below the threshold.
- **A move completes the layout first.** `BookCanvas::finish_layout` runs the rest in one go; it is called by everything
  that needs the whole chapter (search, anchors, characters, `canonical_text`, selection by keyboard, dev hooks) and by
  `settle_for_move` before a page turn or wheel step when a window is shown or the page is within three pages of the end of
  what is laid out. The reader asks `text_sample` (laid out text only) for the language instead of `canonical_text`, and
  marks vocabulary only when `layout_complete()`.
- **Pango is not thread safe in gtk-rs** (`pango::Layout` is neither `Send` nor `Sync`, and the layouts are what the page
  draws from), so "a worker thread" was not an option; slices on the main loop give the same result for input (nothing
  blocks longer than one slice) without a second text engine.
- **Library, 100,000 books**: Select All / click selection (`library.rs`): the `selection-changed` handlers walked all
  100,000 rows for every change and every change was one book (`select_item` per book in `restore_selection` and in each
  piece of the fill: 10^10 steps); the handler now reads the bitset of the selection (`selected_ids`), the selection is
  restored with one `set_selection` and one change per piece; `requery` kept the selection with two `Vec::contains`
  (5 x 10^9 comparisons with everything selected, replaced by sets); `Library::get` / `get_mut` are a binary search
  (books are appended with rising ids; a list out of order or a missing id falls back to the scan) instead of a scan per
  selected book in the details, shelves and device code; the model that is not shown is no longer built (`requery` built
  the 100,000 `BookItem`s and the 100,000 `BookRow`s); the JSON records of the catalogue are parsed on up to 8 threads.
- **Test fixture**: `make_scale_calibre` created a Calibre database without the indexes a real one has, so the importer
  scanned a link table per book (100,000 books took 4 min 26 s, now 9 s); the fixture has the indexes now.

### Numbers

| What (release) | Before | After |
|---|---|---|
| Open the 10 MB book (`big_book_reader`): longest main-loop stall | **591 ms** | 54 ms (the rest is parsing the chapter and filling the 20,020 contents rows); first screen after 7.6 ms of layout |
| The same chapter laid out | 540 ms in one call | 540 ms in about 70 slices of 8 ms, behind the page |
| Open a chapter at 60 % (saved position), first page | after the whole chapter: 540 ms | **4 to 6 ms of layout** (estimate + 16 calibration blocks + window), exact position after 540 ms in the background; estimate of the height 316,797 px against 332,970 px (-4.9 %), position 0.598 against 0.599, page count 587 against 601 |
| Open a 52 MB EPUB (100,000 contents entries, chapters of 2,200 paragraphs) | one stall of about 2.6 s | longest stall 154 ms; 2.6 s of layout in the background; RSS 571 MB |
| Select All, 100,000 books (`library:selectall`) | **6,976 ms** stall | longest stall 61 ms (49, 32, 61) |
| Rebuild the list with all 100,000 selected (`library:requery-time`) | **more than 290 s** (killed at the 300 s limit) | 216 to 280 ms |
| Cold start with 100,000 books: library window with the first books | 509 ms after the process started | **376 ms** (0.19 s for a library of two books); the other 100,000 follow in pieces for 1.25 s without blocking; RSS 600 MB |
| Open the catalogue in the app (`library_health::open`, 100,000 books) | 156 ms | 100 ms |
| Build the items of the grid (`requery`, 100,000) | 114 ms | 60 ms |
| Typing in the search box, 100,000 books (`t`, `ta`, `tal`, `talko`, clear) | not measured | longest stall 91 ms per key |
| Calibre importer on the 100,000 book test database | 266 s | 9.4 s (test fixture, see above) |

Rough timeline of a cold start with 100,000 books (ms after the process started, Broadway, release): `main` 20,
catalogue open 120 to 220, window built 290, first request of the book list 310, first books in the model 376, all
100,000 in the model 1,630.

### Budgets added

- `crates/bookx-core/tests/perf_budgets.rs`: `a_get_per_selected_book_is_not_quadratic` (100,000 `get` and `get_mut`,
  100 ms), `get_finds_books_in_any_order_and_misses_cleanly`.
- `crates/bookx-store/tests/perf_budgets.rs`: `loading_ten_thousand_records_keeps_the_order_on_several_threads` (300 ms),
  the 100,000 book open budget is 500 ms (was 1,500).
- `src/flow_window.rs`, module `perf`: the estimate for a chapter of 50,000 blocks (10 ms).
- Broadway: `big_book_window` (`build-aux/ui-tests-bigbook.sh`: window first, exact position later, a page turn at once,
  stall budget), `scale_library_100k` (`build-aux/ui-tests-scale.sh`: cold start shows all books, Select All stall, rebuild
  with everything selected, selection survives a search). `big_book_reader` keeps its 4 s debug budget (0 ms reported now).
- Everything in the families `big_*`, `reader_*`, `library_*`, `lib9_*`, `lib10_*`, `notes_*`, `grid_*`, `comic_*`, `pdf_*`
  passes (562 checks), and a second pass with `BOOKX_SLICE_FROM=1 BOOKX_WINDOW_FROM=1` (every chapter laid out in slices,
  every saved position from a window; 912 checks over 161 scenarios) passes too.

### Not done / limits

- **The estimate is visible for about a second** after opening a long chapter in the middle: page "351 of 587" becomes
  "361 of 601". The text on the page is the text of the saved position, the page numbers are what moves.
- **A page turn right after opening completes the layout first** (a stall as long as the layout, 0.5 s at 1500
  paragraphs, once); only the first screen is guaranteed not to block. Turning pages in the background would need the pages
  after the window, which do not exist before the paragraphs before them are laid out.
- **Search, highlights jump, text to speech** need the whole chapter and complete the layout when asked for it.
- **Scroll frames** (fps) of the grid at 100,000 books and of the reader were not measured: Broadway has no frame clock,
  and no real GNOME session was run for this change (`library_scroll_20k` exists for 20,000 books).
- **Memory**: 600 MB resident after opening 100,000 books (catalogue 182 MB, the rest the grid models, GTK and the
  fill), 746 to 880 MB after a few rebuilds (allocator, not a leak that was chased). A 52 MB EPUB opened in the reader
  is 571 MB (the contents model of 100,000 entries and the Pango layouts of a chapter). `BookItem` strings could be built
  per piece instead of for the whole list (60 ms and some tens of MB at 100,000 books).
- **PDF open time** was not measured in the app (`bookx-pdf` has its own budgets: 500 pages in 400 ms).
- Closing the window with 100,000 books takes about 0.4 s (the models are dropped).
- `library:selection` (a dev hook) is quadratic by itself (`position` per id); use `library:selcount` for large libraries.
