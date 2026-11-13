# Library and reader UX gaps (ux-gaps pass, 2026-11-13)

Internal notes: what was built, how it is tested, what is not covered. User-facing text is in the wiki
(`shortcuts.md`, `library.md`, `reading.md`, `preferences.md`).

## What was built

| Item | Rows | Files | Tests |
|---|---|---|---|
| Keyboard shortcut customisation | L36, R36 | `src/shortcuts.rs` (registry, overrides, conflicts, JSON), `src/ui/shortcuts_dialog.rs` (Preferences page, capture dialog, live accelerators), edits in `app.rs`, `palette.rs`, `ui/library.rs` (local shortcut controller rebuilt on change), `ui/reader.rs` (key lookup before the built-in keys) | 24 unit tests, UI `shortcuts_rebind` `shortcuts_files` `shortcuts_loaded`, session `shortcuts_real_keys` |
| Shelves | L5, L29 | `src/shelves.rs`, `src/ui/shelves.rs`, book menu, drag source on grid items and table cells | 9 + 1 unit tests, UI `shelves` |
| Library switcher | L11 | `src/libraries.rs`, `src/ui/library_switcher.rs`, `Output::SwitchLibrary` into the Preferences "library folder" path in `app.rs` | 8 unit tests, UI `library_switcher` |
| Jobs panel | L27, C10 | `src/jobs.rs` (hub), `src/ui/jobs.rs`; publishers: import (`library.rs`), conversion, e-mail, news, metadata fetch (`ui/bulkfetch.rs`), scheduled backup (`ui/backup.rs`) | 10 unit tests, UI `jobs_panel` |
| Go to, jump history, printed pages | R24, R26, R42, F5 | `src/goto.rs`, `src/jump_history.rs`, `src/page_list.rs`, `ui/reader_goto.rs`, reader edits, `canvas.rs` (`anchor_fraction`, `view_end_fraction`) | 7 + 6 + 5 unit tests, UI `reader_goto` (a book with a page-list is generated in the scenario file) |
| Image viewer | R44, F7 | `src/image_view.rs`, `ui/image_viewer.rs`, `canvas.rs` (`Kind::Image` keeps its path, `on_image`, `image_at`) | 10 unit tests, UI `image_viewer` |
| Reading profiles, themes | R34, R19, R47 | `src/reading_profiles.rs`, `src/reader_themes.rs`, `ui/reader_profiles.rs`, `Config.profiles` / `Config.reader_themes` | 9 + 5 unit tests, UI `reading_profiles` `reader_themes` |
| Book map | R12, R27 | `ui/reader_map.rs` (uses `bookx-lookup::bookmap`) | 3 unit tests, UI `book_map` |

Scenarios are in `build-aux/ui-tests-ux-gaps.sh` and `build-aux/session/scenarios_ux_gaps.py`.

## Design decisions

* **Shortcuts.** The registry is `palette::COMMANDS` plus three extra actions (`app.palette`, `book.edit-meta`,
  `book.delete`). Only customised actions are stored (`shortcuts.json`, `{"bindings": {action: [accel…]}}`; an empty
  list means "no shortcut"). Application actions collide with both windows, library and reader with their own group
  only. The reader still has its hard-coded key handler: `shortcuts_dialog::handle_reader_key` runs first, and only
  when something is customised, so nothing changes for people who never rebind. A default key of a rebound action is
  swallowed ("disabled"), unless another action took it over. The library keeps its local `ShortcutController` for the
  five actions that must not fire inside text fields (Delete, Ctrl+A, Ctrl+F, Ctrl+O, Ctrl+E); everything else is a
  `gtk::Application` accelerator, which also makes menus show the live keys.
* **Shelves.** They are the store's collections. The order of shelves and of the books on them is not in the schema
  (no position column); it lives in `shelves.json` per library folder. A shelf is shown through the search term
  `shelf:"Name"`, expanded in `LibraryView::visible_ids` before the core search. Smart shelves are the saved
  searches, listed in the same section.
* **Library switcher.** Switching reuses the Preferences "library folder" path (`PrefChange::LibraryDir`): last
  snapshot of the old library, lock released, `library_health::open` takes the lock or opens read-only. The registry
  file is `libraries.json`; the library in use is added at start-up.
* **Jobs.** The hub is a thread-local on the GTK thread; workers report through the channels they already had.
  Cancel is cooperative (`CancelToken`): imports skip the remaining files (reported as cancelled), conversions stop
  between books, fetches set the batch's own cancel flag.
* **Printed pages.** `page_list::marks` reads the nav document's `page-list`; the footer finds the label by asking the
  canvas where each anchor is (`anchor_fraction`) and where the page ends (`view_end_fraction`), because byte offsets
  in the source do not match the laid-out text. `bookx_text::pages::PageMap` is still built (byte offsets) for the Go
  to… validation and the total count.
* **Image viewer.** A click in the middle zone of the page (the "toggle chrome" tap zone) on a block image or a
  float opens it, so the outer zones still turn pages. Fixed-layout books never open it. The viewer does its own
  pan and zoom (a `gtk::Fixed` in an overlay), not a `ScrolledWindow`, because the scrolled window would not grow
  with `set_size_request` under Broadway.
* **Profiles and themes.** A profile only stores the settings it sets (Option fields) and is applied on top of the
  current reader preferences. A book bound to a profile gets it for the reading session; the previous preferences are
  put back in `Reader::shutdown`. A theme is applied as the `Custom` colour mode.

## Not done, and why

* **Send shelf to device (L5).** The Device window sends the selection and has a Kobo shelf *name*; there is no action
  that sends "this shelf". It was skipped as the brief asked (only if the device UI supports it).
* **Page thumbnails (R12/R27).** The map is a chapter strip; rendering a thumbnail of every page needs off-screen
  layout of the whole book.
* **Highlight positions on the book map** are estimates (a highlight stores a character offset, not a fraction).
* **Profile auto triggers (R34)**, per-book theme and background texture (R47).
* **Gesture manager (R36)**: only the keyboard map.
* **Jobs from device sends, the manual backup window, add from the web, device cover sync** are not published yet.
* NCX `pageList` (EPUB 2) is not read; only the EPUB 3 nav `page-list`.

## Caveats

* The reader's own profile of a book is applied by changing the global reader preferences for the session. If the
  application is closed (window close) while that book is open, the saved preferences are the profile's.
* Real mouse drag and drop onto a shelf is not covered by Broadway scenarios (GTK drags need a compositor); the drop
  handler is exercised through `library:shelf=drop=…`.
* The i18n test `every_shipped_language_translates_every_marked_string` needs the new strings ("Book Map", "Reading
  Profiles…", "Reading Themes…") in `po/*.po`; the i18n pass owns the po files.

## Library window, wave 4 (enhancements 14, 15, 17, 24, bug 8)

Pure logic: `src/library_nav.rs` (views, recent, favourites, details mode, click/selection rules, `plain_label`).
GTK: `src/ui/library.rs` (sidebar, details panel, notes view host, book menu, empty-space click), `src/ui/notes.rs`
(`build` instead of a dialog), `src/ui/health.rs` (tabs), `src/ui/organize.rs` / `duplicates.rs` (page builders).

* **Sidebar.** A `GtkListBox` (single selection) for the four views, a scrolled list for shelves, saved searches and
  browse groups, and a list with Library Health pinned at the bottom, inside one `AdwOverlaySplitView` (always shown;
  collapsed below 860sp with a header button that is visible only while collapsed). Picking a view never rebuilds
  the groups (`rebuild_tags` runs on a library change only). The view is saved as `sidebar_view`; favourites as
  `favourites` (book ids, pruned on refresh).
* **Details panel.** Follows the selection (`follow_selection`); when the selection becomes empty the content is
  left alone so it does not flash while sliding out. In a collapsed window it does not open by itself: the selection
  bar gets a *Details* button.
* **Notes view.** Built once, kept in the content `GtkStack`, `reload` on every visit. The `library:notes:*` dev
  commands are unchanged.
* **Health tabs.** One `AdwViewStack` with a `ViewSwitcherBar`; widgets of every tab exist from the start, so the
  `library:health=*` dev commands work whichever tab is visible (`health=tab=NAME`, `health=open-tab=NAME`).
* **Empty-space click.** `attach_empty_click`: a `GestureClick` on the scrolled windows. Whether a book was hit is
  decided with `pick` and the css node names (`child` for grid items, `row`, `header`, `scrollbar`); this depends on
  GTK's node names (GTK 4.14+ tested).
* **Grid size.** `GtkGridView` cells are the row width divided by the column count; tiles stay their natural size
  and are centred in the cell, so the gutters grow and shrink a little with the width; a fixed-gutter, left-aligned
  layout (Files) would need a custom layout manager or a `GtkFlowBox` (no virtualisation, bad for big libraries).
* **Shortcut.** `lib.favourite` (Ctrl+D) is a library-local shortcut: added to `LIBRARY_LOCAL` in
  `shortcuts_dialog.rs`, because application accelerators do not find actions on the library page.

### Not done / caveats

* Touch: the long press toggles a book; a tap while the selection is not empty is still GTK's plain select (no
  separate "selection mode" with check boxes). Not tested on a touch screen.
* The Series window and the author manager stay as dialogs, reached from the sidebar rows (buttons) and the palette.
* Sidebar open/close time was not profiled; it is a property change of the split view with nothing rebuilt.
* Real-session tests cover: context menu size, selection clicks, sidebar, tile reflow. The narrow-window overlay is
  covered in Broadway only.

## Wave 4 (ux-map): book map, per-book looks, shortcuts, tweaks in the popover

Supersedes the "Profiles and themes", "Book map" and "Page thumbnails" notes above.

* **Book map (enh. 18).** `ui/reader_map.rs` is an `AdwDialog` with one `DrawingArea` that draws only the cells in view
  (a grid of pages, `CELL_W` = 22 px per cell; a longer book scrolls by rows or is zoomed out). All rules are in
  `bookx-lookup::mapview` (grid, zoom levels Whole book / Sections / Single pages, bar heights, marker placement,
  chapter boundaries, wheel and key rules, text snippet); 3000+ page books are covered by a unit test. Time per page is
  `bookx-lookup::dwell` (4096 buckets per book, a visit counts at most 180 s, a bucket 6 h, 500 books, `dwell.json`),
  credited by `Reader::dwell_tick` on every position change and flushed at most every 30 s. Pages = the page-list count
  when the book has one (labels from the page-list), else the layout estimate. Not done: a rendered thumbnail of the page
  (the card shows chapter, page and a text snippet), highlight places are estimates, the dwell of a re-flowed layout is
  spread over the pages by overlap, no autoscroll when dragging past the edge (use the wheel).
* **Per-book looks (enh. 19).** `Config.book_looks` (full `ReaderPrefs` per book) and `Config.reader_defaults`: while a
  book is open `cfg.reader` is the book's look and the file is written with the defaults (`file_text`). Old profiles
  migrate at load (`Config::migrate_profiles`, backup `reading-profiles-backup.json`). Custom themes are swatches
  (`ui/reader_theme_row.rs`); the editor is `ui/reader_theme_editor.rs`. Caveat: a look stores every `ReaderPrefs` field,
  so changing a default later does not change books that already have a look. Looks of deleted books are not cleaned up.
* **Shortcuts (enh. 21).** See `docs/shortcuts-research.md`. Bug found on the way: the palette action
  `reader.back` ("Back to the Library") did "jump back"; it now leaves the book.
* **Style tweaks (enh. 22).** The dialog is gone; `ui/reader_style.rs` builds the rows that sit in the text appearance
  popover. Tweaks are always the open book's (`TweakStore.books`); Use as default copies them to `general`, Reset removes
  the book's entry. A book that switched everything off keeps an empty entry when general tweaks exist.
* **Tests.** Unit: `bookx-lookup` `mapview` (18) and `dwell` (9), `reading_profiles`, `config` migration, `shortcuts`
  (5 new), `tweaks`, `reader::t_cycles_through_the_themes`. UI scenarios: `book_looks`, `profiles_migrate`,
  `reader_themes`, `book_map`, `reader_new_keys`, `style_tweaks`. Real session: `book_map_wheel_drag` (real wheel,
  Ctrl+wheel, drag), `new_reader_keys` (M, Ctrl+W).
* **Known failing, not mine:** `i18n` unit tests need the new strings in `po/*.po` (i18n pass).

### Book map redesign (follow-up)

Blocks are rounded; read bars are an accent gradient; highlights are swatches in the stored colour inside the block
(`mapview::cell_summary` keeps the distinct colours per cell, at most 3 plus a dot; underline and strike-through are
lines), notes are pencils in their colour, bookmarks flags, jump history numbered circles with a halo. Contrast: the
pure helpers `outline_for`, `ink_on`, `to_gray` (`mapview`) pick an outline when a colour would not reach 3:1 on the
page colour (light, dark, sepia, black; always in e-ink and high contrast). The card is `.map-preview` in `style.css`
(solid `@popover_bg_color`, border, shadow; white with a black border in e-ink) and placed by `mapview::card_origin`
(flips and clamps). Zoom changes fade in over 220 ms (not with e-ink or reduced motion), RTL mirrors the columns and the
arrow keys, lines are snapped to device pixels. Screenshots checked by hand from the real session
(`book_map_look`, `book_map_look_dark`, `book_map_look_contrast`; images in the session results folder), the wiki copies
are `wiki/docs/assets/book-map*.png`. Not covered: e-ink and RTL have only unit tests (no screenshot), the sepia and
black reader themes do not change the dialog (it follows the application style), and chapter titles are shown only at
the single-page zoom.

### Library window, wave 4 follow-ups (split header, sidebar look, notes cards)

* **Split header.** The window page is one `AdwOverlaySplitView`: the sidebar is an `AdwToolbarView` with its own
  `AdwHeaderBar` (add, library switcher, search, main menu; no window buttons), the content an `AdwToolbarView` with
  its own header (view name + count, jobs, view options, device button, window buttons). `header` in `W` is the content
  header. The sidebar toggle of the content header shows only while the sidebar is collapsed.
* **Sidebar look.** CSS under `.bookx-sidebar` in `style.css` (36 px rows, 8 px radius, group headers small and
  bold, hover-only `.row-actions`, placeholder row). It still uses `AdwExpanderRow` for the groups, so the
  selectors depend on libadwaita's node names (checked with the real-session screenshots, libadwaita 1.7).
* **Library Health icon.** `security-high-symbolic` (Adwaita, status). `emblem-ok-symbolic` does not exist in the
  Adwaita icon theme and was replaced everywhere (Health tab, duplicates, preflight, editor tools). The
  `icons_exist` scenario (`library:icons`) walks every window for icon names missing from the theme.
* **Notes cards.** `src/ui/note_row.rs` (`NoteCard`, `NoteData`, `NoteActions`) is the shared card; the library view
  uses it. The reader sidebar (`reader_side.rs::build_note_row`, owned by the reader agent) still has its own copy of
  the same layout: switching it to `NoteCard` is a small follow-up (its menu callbacks map to `NoteActions`). CSS for
  both: `.note-card`, `.note-block`, `.note-row` in `style.css`.
* **Date filters.** Two hidden `GtkEntry`s still hold the `YYYY-MM-DD` text (the filter and the `library:notes:from=`
  test command read them); the visible controls are calendar popover buttons ("Any time" clears).
* Real-session screenshots (light and dark) of the library, the selected book and Notes & Highlights:
  `scenarios_library.py::library_look_light/dark`, files in `$CARGO_TARGET_DIR/session-results/`; the wiki has
  `library-details.png`, `library-notes.png`, `library-notes-dark.png`.

### Book map thumbnails and marker geometry (follow-up)

Hover card = a thumbnail of the real page (`ui/map_thumbs.rs`, `bookx-lookup::thumbs`: `locate`, `Lru`, `Latest`,
`Debounce`, `thumb_size`; `BookCanvas::{style, view_size, set_offscreen_size, render_thumbnail}`), the page uses the
reading theme, the card the app style. Works for EPUB (horizontal and vertical writing; fixed layout through the same
canvas style). **PDF: not done** - the PDF reader has no book map, so there is nothing to hang a PDF thumbnail on.
Not sliced: the layout of one chapter (one main-loop chunk). Highlight lines now span the whole block (clipped to its
rounded shape), notes are pencils under the block (`mapview::{highlight_line, pencil_centre, pill_rect, on_pencil}`,
row gap 16 px) and the card shows the note's text when the pointer is on its pencil.

### Continue Reading banner (adaptive) and compact Notes view

* `library_nav::banner_plan` (viewport → slim/hero and heights), `banner_geometry` (scroll offset → card height,
  visible height, slide) and `banner_scroll` are pure and unit tested; `src/ui/library_banner.rs` (`BannerCtl`)
  puts them on the widgets. Slim: at most 25 % of the viewport and 120 px (with margins); hero from a 1100 px
  viewport: 32.5 %, between 900 and 1100 px the height grows smoothly and the form switches at the middle.
* Scrolling: the wheel (capture phase on the book page) is spent on the banner first: it shrinks to the slim card,
  then slides off; the books do not scroll meanwhile. Keys, the scroll bar and touch-drag move the books directly and
  take the banner away at once. No height animation exists to switch off; the hero/slim content switch is a
  crossfade stack (none in e-ink mode; GTK disables it with reduced motion). The room is read from the adjustments
  of the book list (`changed`), so window, sidebar and details panel toggles replan; `library:viewport=HxW` forces a
  size for Broadway tests (a Broadway window cannot be big).
* Notes cards: `NoteCard::compact`, one `FlowBox` per book; the list uses the whole width. The chapter is on each
  card's meta, there are no chapter sub headings any more.
* Not measured: smoothness of the wheel while the banner collapses on a real touchpad (kinetic scrolling is not
  intercepted: it moves the books).

### Continue Reading banner, redesign (hero on a cold start, strip afterwards)

Decision: **variant A** of the mockup sheet (`mockups/bookx-banner.png`, `examples/mockups/banner.rs`): hero = big cover |
text, progress, chapter, time left, Continue + Details / Open from start / Mark finished | "Also reading" covers.
B (cover leaning out of the card) overflowed the card and clipped on small areas; C (progress ring) hid the title
space and carried only one action.

* **Session behaviour** (`library_nav::banner_kind`, `HeroSession`): the first banner of a run is the hero and stays while
  the visit goes on; opening a book or choosing another view ends the visit; later visits get the strip, the hero
  returns after 30 minutes (`HERO_AWAY_SECS`). Kept in memory (a new run = a cold start), not in the config.
* **Layout** (`banner_plan`, pure): width of the book area decides the layout (wide ≥ 900, medium ≥ 640, strip ≥ 420, mini),
  the viewport the height. The hero needs 232 px and may take at most a quarter of a small viewport, so a viewport lower
  than 928 px gets the strip even on a cold start; from there it grows to 32.5 % (cap 520 px). The strip is 64 px
  (88 with margins). Heights are animated for 200 ms (`adw::TimedAnimation`, off in e-ink or without GTK animations); the
  layouts cross-fade in a `GtkStack`. Scrolling shrinks hero to strip, then slides it off; the books do not move while it does.
* **Colours** (`src/cover_color.rs`): dominant colour of a 32x48 thumbnail weighted by saturation, saturation capped
  (0.55 light, 0.45 dark), pastel gradient in light mode, deep in dark mode; the text is black or white by WCAG contrast
  (tests check 4.5:1 on both gradient ends for 432 colours in both modes). Applied by a `CssProvider` on the `themed` class.
* **Not done:** the time left is a rough guess from the file size (`time_left_text`, "about"), wrong for image-heavy books;
  the chapter shown is "Chapter N" from the saved position, not the book's title for it (that needs the book opened).
  Touch drag-scrolling is not intercepted, it moves the books directly.
* Tests: unit `library_nav` (session, ladder, 25 % rule, geometry), `cover_color`; UI `banner_session`, `banner_ladder`
  (12 widths and 12 heights through `library:viewport=`); session `library_hero_light/dark` (a real pointer drags the window
  corner: 900x600, 1280x800, 1920x1080, 2400x1300, details panel open, back; full width, not a square, share of the
  viewport), `library_hero_session` (hero, then strip after opening and closing a book).

### Favourites, rating and tags (enhancement 25)

`src/tags.rs` (normalise, add/remove, union counts, suggestions), `src/ui/meta_edit.rs` (`rating_stars`, `tag_editor`),
`Library::edit_meta` in `ui/library.rs`. An edit made in the details panel redraws the books but not the panel (it keeps
focus); an edit from Properties or a script redraws both (`Input::SetRating/EditTags { in_panel }`).
`metadata_fetch::changed_fields` now includes `rating` (provenance of a rating). Not done: half stars; a rating is not
written into non-EPUB formats (`update_metadata` fails for them and nothing changes).

### Thumbnails are the reader's page (proof) and fast (follow-up 2)

* **Faithful.** Real session, `book_map_faithful_{light,dark,alice,vertical}` (`build-aux/session/scenarios_map_keys.py`):
  jump to page N with the map, screenshot the reader, reopen the map, hover the same block, cut the thumbnail out of the
  screenshot, scale the reader's page (the half of the view that holds it) to the thumbnail size and compare. Also asserts
  that the thumbnail's chapter / first page of its view / pages of the chapter equal the reader's. Results (mean absolute
  error 0..255 after a light blur and the best of +-6 px shifts; dHash distance 0..64; the control is a page of another
  place): big book light 2.7 / 11, 0.8 / 1, 2.8 / 6 (control 10); dark 3.3 / 8, 1.0 / 1, 3.5 / 3 (control 13); Alice
  sepia 3.0 / 3, 2.5 / 7, 2.7 / 4 (control 7.8 - 9.5); vertical synthetic 3.1 / 6, 3.6 / 4, 1.3 / 5 (control 10 - 13).
  Thresholds: MAE <= 9 and dHash <= 20. Three pages in different chapters each. Causes found on the way: the reader adds
  1e-6 to a place it jumps to (`thumbs::JUMP_EPSILON`, now mirrored by `locate`); one thumbnail = one page of the view
  (a half), not the whole two column screen.
* **A bug the tests found:** while the map was open the pointer handler answered GTK's repeated "same position" motion
  (it repeats after the card moves), which re-placed the card, which made GTK repeat the motion: hundreds of redraws a
  second and the debounce never settled. Same-position motions are ignored now and the card is only moved when its
  place changes.
* **Not done:** a PDF thumbnail (the PDF reader has no book map), the windowed layout (see `docs/perf.md`), the
  unsliceable chapter layout. Fixed layout EPUBs use the same canvas style but have no screenshot test.
