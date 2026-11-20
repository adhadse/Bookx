# Comic reader notes (PP-G-24, roadmap R52)

Internal notes: how it is built, what was tried, caveats. The user-facing text is the wiki page *Reading → Comics and manga*.

## Design

* A comic is imported as an EPUB with one `images/page-NNNN.ext` per page (`bookx-formats/src/comic.rs`). The converter now adds
  `<meta name="bookx:comic">`, `bookx:comic-double` (ComicInfo `DoublePage`) and `bookx:comic-types` (page types other than Story) to the
  OPF (`bookx_comic::meta`). Books imported before the marker existed are recognised by their first chapter (nothing but a page image).
* `Msg::BookLoaded` in `src/app.rs` asks `crate::comic::open_comic`; a comic goes to `ui/reader_comic.rs` (a component of its own, like the
  PDF reader), everything else to the EPUB reader. `reader.rs` was not touched.
* Everything that can be decided without a window is in `crates/bookx-comic`: `decode` (limits, reduction, grey, panels), `panels`
  (XY cut on the gutters), `smart` (double-click target, guided stops), `layout` (spreads, strip, direction, click zones), `view` (camera),
  `cache`, `worker`, `settings`. The UI file draws with cairo, handles input and keeps the state.
* Camera model: a point of the content (picture pixels of a page, a spread or the whole webtoon strip) is at `content * s + (x, y)` on
  screen. Panel zoom is `view::focus`, animation is `view::tween` (centre moves linearly, scale geometrically) driven by a tick
  callback from real time, so a slow frame clock only drops frames.
* Decoding: 2 threads, a queue that the reader replaces whenever it moves (pages on screen first, then 3 ahead in the reading direction
  and 1 behind, thumbnails last). A page is reduced to 4096 px on its longest side and 12 megapixels (2400 px in e-ink mode); the cache
  keeps 320 MiB of decoded pages and never drops what is on screen. Panels are found on the reduced page on the worker thread, so a
  double-click never waits for detection (when a page is not decoded yet it waits for the page).

## Panel detection

Border colour (or white, or black) is the gutter colour; a line of the page is gutter when 99.6 % of it is within 28 grey levels; gutters
narrower than 0.9 % of the page are ignored; cuts alternate between rows and columns; leaves narrower than 6 % of a side are dropped;
a single leaf covering more than 95 % of the page means "no panels". Tests use generated pages only (`bookx_comic::testgen`).
**Not tried on real comics.** Known limits: panels that overlap or are slanted, speech bubbles that cross gutters, halftone backgrounds that
reach the page edge, gutters that are not one colour, pages with a frame drawn around everything. The reader falls back to the whole
page or to a region around the pointer, so a miss costs a double-click, not a broken page.

## Settings

`~/.config/bookx/comics.json`: `global` (defaults: layout, fit, cover alone, wide alone, guided, grey, animate, thumbnails) and `books`
(per book id: layout, fit, direction, guided). Changing layout, fit or direction in the reader stores them for that book only; the
checkboxes for cover / wide / grey change the global value. *Use the defaults for this book* removes the entry. Preferences → Reading →
Comics edits the global part. A book's direction falls back to ComicInfo (`Manga=YesAndRightToLeft`).

## Tests

* `cargo test -p bookx-comic` (63 unit tests), `crates/bookx-comic/tests/perf_budgets.rs` (decode + reduce + panels of a 4000 x 6000
  page, panel detection alone, sizes + layout of 500 pages, preloading six pages on two threads), `-p bookx-formats comic`, `-p bookx comic`.
* Broadway: `build-aux/ui-tests-comics.sh` (scenarios `comic_*`; hooks in `_developer/docs.md`). Broadway's frame clock runs at
  a few frames per second, so the scenarios switch animations off (`comic-animate=off`) except the one that checks that panel moves are
  animated.
* Real session (real double-click, keys, drag, wheel, screenshots): `build-aux/session/scenarios_comics.py` (`comic_smart_zoom`,
  `comic_manga`, `comic_modes_input`). The panels have different tints, so the scenarios read the screenshot to tell which panel is
  shown. The real session found one bug the Broadway hooks could not: a swipe also fired the click handler and hid the bars.

## Not done / caveats

* **Next volume in the folder**, series from folder names, per-library direction, "ignore ComicInfo" switch: not built.
* **CBR** (real RAR): still rejected with a message; no pure-Rust RAR decoder.
* Pages are shown with cairo's bilinear / "good" filter from the reduced bitmap; no sharpening, no gamma, no auto-crop of white margins,
  no contrast slider for e-ink. A page is decoded once per size cap; zooming to 6x shows the reduced bitmap enlarged (4096 px cap).
* The thumbnail strip is the comic's own sidebar, not the EPUB *Book map* (that window is for text books). Thumbnails are decoded
  on demand for the rows in view, not stored on disk.
* Reading statistics count a turn as a page of 250 words (the PDF reader does the same); no comic-specific statistics.
* No text selection, search, highlights or notes in comics (nothing to select). Bookmarks of the EPUB reader are not offered.
* Not tried on a touch screen or e-ink hardware: swipe and pinch are wired but only the code path is tested (drag hooks in Broadway,
  a real mouse drag in the session).
* Memory: a 4096 x 2730 page is 45 MB decoded; the budget holds about seven of them. A webtoon whose pages are all very long may keep
  fewer pages than it needs for smooth scrolling on small machines.
