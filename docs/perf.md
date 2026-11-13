# Performance budgets

Budgets live in `crates/*/tests/perf_budgets.rs` (and `perf` modules in the `bookx` binary crate, for code that is
not in a library). Each asserts that an operation stays under a budget in release builds. Debug builds get 30x room;
`BOOKX_PERF_SCALE=2` scales every budget. Run them in release with `make perf`.

## Hot path coverage

| Hot path | Where |
|---|---|
| EPUB / KEPUB / MOBI / AZW3 / other writers on a 100-chapter book | `bookx-convert/tests/perf_budgets.rs` |
| Full-text index build (2,000 texts of 300 words) and search | `bookx-store/tests/perf_budgets.rs` |
| OPDS feeds of a 10,000-book library (root, all first/last page, new, authors, series, search) | `bookx-serve/tests/perf_budgets.rs` (new) |
| Hyphenation of 10k words, line breaking, style computation | `bookx-text/tests/perf_budgets.rs` |
| Editor search/replace over about 1.2 MB of XHTML in 200 files, file scoping | `src/editor_search.rs`, module `perf` (new) |
| Library search, sorted listing, load, save, import of 10k books | `bookx-core/tests/perf_budgets.rs`, `bookx-store/tests/perf_budgets.rs` |
| Validation, reports, compare, spell check of a 5 MB book | `bookx-editing/tests/perf_budgets.rs` |

## Measured numbers

Machine: AMD Ryzen 9 9950X (32 threads), rustc 1.93.1, `cargo test --release`, one test at a time
(`--test-threads=1`), 2026-11-12. The figures are the **wall time of the whole test**, including building its
synthetic data, not only the budgeted call; the budgets themselves are the per-operation limits in the source. Every
budget test passed. Most budgets are generous (10x to 100x over the measured time) so they catch regressions without
failing on slow CI machines; tighten them with `BOOKX_PERF_SCALE=0.1` to look for creeping slowdowns.

| Test | ms (whole test) |
|---|---|
| `a_patch_over_a_large_book_is_fast` | 31.7 |
| `aggregating_20k_sessions_is_fast` | 2.2 |
| `breaking_a_300_word_paragraph_is_fast` | 0.8 |
| `breaking_a_whole_chapter_stays_linear_enough` | 8.0 |
| `catalogue_of_10k_books_loads_fast` | 27.7 |
| `chm_lzx_book_converts_within_budget` | 74.6 |
| `compare_on_a_5mb_book` | 91.8 |
| `computing_styles_for_1000_elements_is_fast` | 2.6 |
| `cover_generation_and_processing` | 7.4 |
| `duplicate_detection_in_a_large_library` | 107.7 |
| `endless_loop_is_cut_off_quickly` | 3.7 |
| `epub_and_kepub_of_a_large_book` | 11.7 |
| `ereader_pml_book_converts_within_budget` | 31.2 |
| `feed_parsing_and_article_extraction` | 17.3 |
| `hyphenating_10k_words_is_fast` | 2.4 |
| `importing_books_scales_across_cores` | 4.5 |
| `loading_a_plugin_is_cheap` | 0.6 |
| `mobi_and_azw3_of_a_large_book` | 226.8 |
| `office_formats_of_a_large_book` | 32.4 |
| `opds_feeds_of_a_10k_library` | 9.9 |
| `opening_a_book_is_fast` | 0.7 |
| `palm_and_rocket_formats_of_a_large_book` | 46.1 |
| `palmdoc_compresses_a_megabyte_quickly` | 1.6 |
| `rb_book_converts_within_budget` | 12.4 |
| `reopening_a_10k_library_is_fast` | 91.6 |
| `reports_on_a_5mb_book` | 159.0 |
| `saving_a_10k_library_the_first_time_is_one_transaction` | 59.5 |
| `saving_one_edit_among_10k_books_is_fast` | 70.9 |
| `saving_only_writes_what_changed` | 31.2 |
| `search_10k_books_is_instant` | 4.8 |
| `searching_a_megabyte_of_text_is_fast` | 0.6 |
| `smartening_a_megabyte_of_markup` | 7.4 |
| `snb_book_converts_within_budget` | 15.5 |
| `snb_of_a_large_book` | 43.7 |
| `sorted_listing_of_10k_books_is_instant` | 6.4 |
| `spell_check_on_a_5mb_book` | 132.1 |
| `structural_model_of_a_large_book` | 11.8 |
| `tcr_book_converts_within_budget` | 21.9 |
| `template_rendering` | 13.4 |
| `ten_thousand_books_put_and_metadata_search` | 105.7 |
| `text_search_over_two_thousand_texts` | 90.2 |
| `thousand_echo_calls_with_payload` | 1.9 |
| `thousand_trivial_calls` | 1.3 |
| `validate_a_5mb_book` | 230.9 |
| `zip_and_oeb_of_a_large_book` | 5.5 |
| `editor_search::perf::searching_and_replacing_across_a_large_book (bin `bookx`)` | 7.9 |
| `editor_search::perf::scoping_thousands_of_files_is_instant (bin `bookx`)` | 1.1 |

Not measured this way: the UI performance scenarios in `build-aux/ui-tests.sh` (`reader_budget`), which run in a debug
build under Broadway.


## Book map thumbnails

`ui/map_thumbs.rs`. A thumbnail is **one page** (half of a two column view, else the whole view), laid out by an
off-screen `BookCanvas` of the reader's size and style, so it is the page the reader shows (proved pixel-wise in the
real session, see `docs/ux-gaps-notes.md`). GTK objects are not `Send`, so the work is time-sliced on the main loop:

* hover: 60 ms debounce, `HIGH_IDLE` priority, outdated requests dropped before they start;
* prefetch when the map opens and when the view moves: the 40 pages nearest the pointer, chapter by chapter (at most
  3 chapters laid out ahead), one page per slice at `LOW` priority, resting as long as the slice took, and a layout is
  not started while the pointer moved in the last 150 ms;
* the layouts of the last 3 chapters are kept; 64 thumbnails in an LRU; a size-capped (64 MB) PNG cache on disk
  (`~/.cache/…/map-thumbs/<book>/<layout>/<page>.png`, the folder name covers the book file and date, size, style,
  language, highlights, style sheet and device scale), so a second map is instant;
* until the page is ready the card shows the nearest page that is (dimmed), else a skeleton page; never a spinner.

Budgets, measured by the scenario `map_thumb_perf` (`PROFILE=release build-aux/ui-tests.sh`; a debug build gets 12x):

| | budget (release) | measured release | measured debug |
|---|---|---|---|
| first thumbnail of a chapter, layout included (Alice, 21 page chapter) | < 150 ms | 78 - 90 ms | 247 ms |
| a thumbnail in the cache | < 5 ms | 0 ms | 0 ms |
| another page of a laid out chapter | < 40 ms | 1 - 3 ms | 4 ms |
| 40 pages nearest the pointer made ahead (incl. 4 chapter layouts) | < 2.5 s | done within 2.5 s | within 30 s |
| longest prefetch slice that only paints or reads the disk | < 16 ms | 3 - 5 ms | 6 ms |
| a page from the disk cache in a second map | < 15 ms | 2 ms (50 hits) | 44 hits |
| longest prefetch slice that lays a chapter out | not bounded | 158 - 205 ms | - |

Where the time goes (release, Alice chapter): layout 76 ms, moving to the page < 1 ms, painting 1 - 2 ms (the page is
painted scaled, not full size and shrunk), counting ink < 1 ms. **The layout of one chapter cannot be cut into slices**
(the canvas builds a chapter's whole flow in one call), so a prefetch slice that lays out a long chapter holds the main
loop for its duration (up to about 200 ms for the longest chapters of Alice in a release build); that is why layouts
only start while the pointer is still, and why a laid out chapter is kept. A windowed layout around the target page was
not done: the layout has no entry point that starts in the middle of a chapter, and the page numbering needs the whole
flow.
