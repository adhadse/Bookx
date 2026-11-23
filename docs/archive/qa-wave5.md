> Archived 2026-11-23: Finished wave log; its open items (grid rebuild, Notes stalls) were fixed in wave 6 (`docs/performance.md`), the rest are tracked in remaining.md. Living document: [remaining.md](../remaining.md).

# QA pass, wave 5/6 (internal)

Run on the tree at origin/main 2cc9fe7 plus other agents' uncommitted work, own target dir `~/.cache/target-qa`.
Sandbox libraries and the public-domain books in `~/.cache/bookx-testbooks` only.

## What was run

* Real GNOME session suite (`build-aux/session-tests.sh`, `KEEP=1 JOBS=3`): 120 scenarios. First run 77 ok, 42 "no result":
  41 of them were `ModuleNotFoundError: scenarios_works` (another agent added the import while the run was going), the
  42nd was `tts_player_card_real` ("the X stops reading and hides the card" failed once under load). Re-run of those 42:
  all ok (screenshots in `~/.cache/target-qa/session-results2/`, first run in `.../session-results/`).
* Broadway suite (`build-aux/ui-tests.sh`, 416 scenarios): 2675 checks passed, 11 failed. 6 are comics (`comic_memory`,
  `comic_prefs`) and 2 vocabulary (`vocab_list_filters`, `vocab_a11y`): other agents' work in progress, not touched.
  The other 3 were mine/found here and are fixed (below).
* Screenshots looked at: library at 360x294 (German, light), 700x500 (German, dark, sidebar), 900x600 hero (dark), jobs
  panel German 900x600, TTS card + speed-reading card side by side (no overlap), Editor Preferences. Layout was clean in
  those (no clipping, no overlap, dark colours right).
* Console: every `*.app.log` of the session run grepped for `CRITICAL|WARNING|panicked|Gdk-Message`.

## Found and fixed

1. Library header subtitle was built with `format!` ("2 books", "1 of 2 books"): never translated, and "1 books".
   Now `ntr`/`trf` (`src/ui/library.rs`). Test: `library_subtitle_translated` (`build-aux/ui-tests-i18n.sh`, de and es).
2. Jobs panel: "130 of 132 MB" and "about 2 s left" were English in every language (`src/jobs.rs`); the banner's
   "about 25 min left" too (`src/library_nav.rs::time_left_text`). Now `trf("{} of {} {}")` / `trf("about {} left")`
   with de/es/fr entries. English unit tests of both still pass; the German popover was seen in
   `library_jobs_german_small` (screenshot `session-results2/library_jobs_german_small-jobs-light-de-900x600.png`).
3. Panic at shutdown: `called Result::unwrap() on an Err value: Requery` (relm4 `emit` on a library component that is
   already gone when the window closes with a reader open; seen in `shortcuts_loaded`, which failed its "no errors"
   check). `src/app.rs` `ReaderPopped`/`AudioPopped` now ignore a failed send. Regression: `shortcuts_loaded` (fails before).
4. Session harness asked for `GSK_RENDERER=ngl`, which GTK now warns is renamed (`Gsk-WARNING` in every log). Now `gl`
   (`build-aux/session/harness.py`).

## Open (not fixed)

* `Gtk-WARNING: Trying to snapshot GtkScrolledWindow without a current allocation` once per run in `library_hero_light`
  and `library_hero_dark`, right when the window goes from the wide hero to the strip (resize). Probably the chips
  strip's scrolled window shown before allocation; library grid area, left to the works-grouping agent.
* The library title and subtitle are not re-translated when the language is switched at run time (only on the next
  refresh, e.g. a search). The sidebar and menus are. Minor; start-up language is right.
* `tts_player_card_real` flaked once with 3 sessions at once (passes alone and in the re-run). Timing, not reproduced.
* `Gdk-WARNING: Duplicate call to gdk_frame_timings_presented()` in 27 logs: a headless-shell compositor quirk, not Bookx.
* Comic and vocabulary Broadway failures, plus the `i18n` unit test (es lacks the new vocabulary strings) and
  `library_nav::no_label_in_the_sources_ends_in_an_ellipsis` (new "…" labels in `audiobook_player.rs` and
  `vocab_list.rs`) and `audiobook::tests::listening_time_goes_into_the_statistics...`: other agents' unfinished work.
  `cargo clippy -D warnings` also reports dead code in `annotation_sync*.rs` (not mine).

## Not covered in round 1 (done in round 2 below)

Walked only through the existing scenarios and the screenshots above, not hand-driven: Preferences pages one by one in
German at 900x600, first-run empty library, vertical writing/PDF visual review (their scenarios pass; PDF skipped: test
PDF not available), editor Tools dialogs by eye, plugin run, Add Narration. The slow-action (>200 ms) check was not done.
Network scenarios skipped (`NET=0`).


# Round 2 (by eye, real session)

New throw-away-turned-permanent scenarios in `build-aux/session/scenarios_qa.py` (imported by `scenarios.py`):
`qa_prefs_{900,1280}_{light,dark}_{en,de}` (every sidebar page of Preferences: clicks, screenshot, no row sticks out of
the dialog, every control named, no text squeezed into a narrow column, no English leftovers in German),
`qa_firstrun_{900,1280}_*` (empty library: welcome page, Recent/Favourites/Notes views, German/Spanish),
`qa_editor_dialogs_*` (Set Cover, Manage Fonts, Transform HTML, Book Jacket, Accessibility Metadata, Set Semantics,
Check External Links, Add Narration with the mock engine; labels of each dialog are saved to `*.labels`),
`qa_stalls_2000` (needs `BOOK_BIG`, a generated 10 MB EPUB: `build-aux/make-big-epub.py OUT 20000`; skips without).
The harness now also fails any scenario whose app log has a GTK/GLib CRITICAL (`harness.py`), and `APP_G_DEBUG=fatal-warnings GDB=1`
gives a backtrace of the first warning of the app only.

## Found and fixed in round 2

1. **German Preferences were full of English** (found by comparing the labels of the `de` and `en` runs): colour scheme,
   layout, tap zones, touch UI, mail encryption, footer item names and presets, shortcut names and group titles ("Application",
   "Not set"), the device-sync switches and help text, the Library files, Plugins, Patches and Devices descriptions, "Metadata
   lookup" description, MCP "no assistant", the keyring caption, the whole privacy ledger ("sends …", "not stopped by offline
   mode"), library sidebar groups (Languages, Reading status, Ratings, Publishers). Cause: strings were either not marked
   (`tr`/`N_`) or returned by `label()` functions without a marker. About 110 new de/es entries (fr for a few). Test: the German
   run of `qa_prefs_*` checks 12 phrases are gone.
2. **Editor tool dialogs in German**: title, intro and Apply button of every dialog built by `tool_dialog` (Spec), crop and size
   choices of Set Cover, rule kinds of Transform HTML/Styles, Set Semantics names, font embedding messages, Book Jacket action,
   the file-group headings and tabs of the editor sidebar (Text/Styles/Images/Package, Files/Contents/Metadata), Live CSS,
   several tooltips. The editing crate cannot call `tr`, so its `label()` names are listed in `src/crate_labels.rs`; a unit test
   (`every_label_of_the_editing_crate_is_listed_for_translation`) fails when a new label is not listed.
3. **Shortcuts page, Backup row** (German 900 px): the explanation was squeezed into a 60 px column of 8 lines by the three
   buttons. Now a tooltip. The check "no text squeezed into a narrow column" in `qa_prefs_*` covers every page.
4. **First run**: Recent and Favourites in an empty library showed the *Welcome* page. They now say what fills them
   ("Nothing read yet", "No favourites yet"); the welcome text no longer claims only EPUB/text/Markdown/HTML; the News
   button tooltip was not translated. Test: `qa_firstrun_1280_light_en` checks the texts of the four views.
5. **Library title/subtitle on a runtime language switch** (item 5): `language_settings::on_change` listeners; the library
   re-queries (title "Start | 2 Bücher" at once). Test: Broadway `library_subtitle_translated`.
6. **`Gtk-WARNING … snapshot GtkScrolledWindow without a current allocation`** (hero → strip): the banner re-planned (changed
   height) inside the `changed` signal of the book list's adjustments, which fires while it is being allocated. Now deferred
   to a `HIGH_IDLE` source. Test: `_hero_case` checks the log for the warning (gone; it was in both hero runs before).
7. **`Gtk-CRITICAL gtk_widget_is_ancestor` in the works scenarios** (details panel): the selectable title label was the first
   thing a focus move reached (GTK bug in `gtk_label_grab_focus`). `set_focusable(false)` on that label (the mouse still selects).
   Test: the harness-wide CRITICAL check (`works_editions_light_wide` failed with it before; backtrace via `APP_G_DEBUG`).
8. **Dev hooks added**: `prefs:pick=#N`, `library:sort=N`, `BOOKX_STALL_WATCH=MS` (main-loop stall reporter, see
   `src/devtools.rs`: `stall: 340 ms after library:sort=1`).

## Main-thread stalls (item 4): 2000 books (Calibre import) + a 10.3 MB EPUB with 20 000 contents entries

Release build (`target-qa/release`) in the real session, stretches of the main loop that did not run (a command can cause two:
the work and the first frame afterwards). The debug build gave almost the same numbers, so it is not debug overhead:

| Action | Stall |
|---|---|
| sort by another key / back | 930 + 500 ms / 860 + 490 ms |
| search "ba" / clear the search | 480 + 295 ms / 810 + 500 ms |
| Notes & Highlights view | 1030 ms |
| Library Health | 150 + 140 ms |
| Preferences | 230 ms |
| open the 10 MB book in the reader / close it | 1140 ms (+ two of 60-70) / 440 ms |
| open it in the editor | 2590 + 1120 + 1470 + 1990 + 400 ms (about 7.5 s in total) |

Analysis: every `requery` of 2000 books spends ~160 ms in `grid.clear()` and ~680 ms in `grid.extend_from_iter()` (relm4
`extend_from_slice` on the `ListStore`; the book data are built in 2 ms). GtkGridView realises its tiles inside that call, so the
cost is the tile widgets (an overlay with six overlays, buttons, labels each; `setup`/`bind` in `BookItem`). Not a simple fix:
options are fewer widgets per tile (build the badges lazily), recycling the existing tiles when only the order changes
(`ListStore::splice` through a wrapper), or skipping the requery when the visible ids did not change. Left open (library grid is
the works-grouping agent's area, now finished: someone should own it). The reader and editor stalls of the 10 MB book were not
analysed further (editor: whole-book work on the main thread in several steps; the file list alone has 40 000 rows of that book
type). Open Notes (1 s) and the search/sort requeries are the ones users will feel.

## Preferences, first run, editor dialogs, new features: what was looked at

* Preferences: 13 sidebar pages (General … KI/MCP) × 900/1280 × light/dark × en/de, 56-84 checks per run, all green after the fixes
  above. Looked at in German light and dark at 900x600 (`~/.cache/target-qa/qa2/*-p*.png`, contact sheets `/tmp/opencode/s-*.png`):
  no clipped labels apart from the language combo "Systemsprac…" (ellipsis of the current value in the combo; fine).
* First run: `~/.cache/target-qa/qa5/` (1280 light en / dark de), 900x600 welcome page in en/de/es. The 900 px sidebar is an overlay, so
  the Recent/Favourites/Notes views were only clicked at 1280. No "tips" exist in the app (nothing to check).
* Editor dialogs: `~/.cache/target-qa/qa6/` (900/1280, light en, dark de). Check External Links shows only a toast when offline
  mode is on (as designed). *Open a Book of Another Format to Edit* opens a native file dialog (portal), which the harness cannot
  drive: not covered. Plugin run: covered by the Broadway scenarios only.
* New features: works (3 session scenarios + keyboard), comics (4), audiobook player (3), own vocabulary (5 + words page) all pass in
  the real session; no CRITICAL/WARNING except the two fixed ones; Preferences pages of KOReader sync (Sync, Share Library) and the
  offline switch (Privacy) were part of the sweep (no overflow, named controls). Not hand-driven: citations export dialog, Share
  Library window.

## Open after round 2

* Grid rebuild and Notes view stalls above: **fixed in wave 6** (`docs/performance.md`: sort 997→24 ms, search 448→23, clear 782→21, Notes view no stall; the cause was the 1281 tiles GtkGridView keeps alive at `max_columns(40)`, plus filling the model in one call); editor/reader open of a 10 MB book.
* `calibre_export` fails on this machine only because a real Calibre process is running (`calibredb` refuses a second client).
* `comic_progress` flaked once in a full Broadway run (load), passes alone.
* Other agents' unfinished gates: `audiobook::tests::listening_time_goes_into_the_statistics…`,
  `library_nav::no_label_in_the_sources_ends_in_an_ellipsis` (the "…" labels of `audiobook_player.rs`/`vocab_list.rs`), clippy dead code in
  `annotation_sync*.rs`.
* Session runs at 3 at once are capped by inotify (the first attempt of a batch sometimes dies with "Too many open files" in the
  sandbox's dbus: re-run those names; not an app bug).
