# Roadmap status (audit of the code, not the progress log)

Audited 2026-11-13, every PARTIAL and MISSING row re-audited 2026-11-22 after waves 6-11 against `docs/bookx-feature-roadmap.md` Parts 1 and 3 (E-series rows are in `docs/archive/calibre-editor-gap-analysis.md`). Status was taken from the code, the tests, the wiki and the UI scenarios, not from the progress log. Rows whose area another agent was working on that day say so.

Status: living | Last reviewed: 2026-11-23 | Owner: @adhadse

## Contents

- [Counts](#counts)
- [A. Reader (R1-R20, R65)](#a-reader-r1-r20-r65)
- [A2. Reader additions (R21-R64)](#a2-reader-additions-r21-r64)
- [B. Library (L1-L13)](#b-library-l1-l13)
- [B2. Library additions (L14-L38)](#b2-library-additions-l14-l38)
- [D/D2. Conversion (C1-C21)](#dd2-conversion-c1-c21)
- [E. Devices and sharing (P1-P21)](#e-devices-and-sharing-p1-p21)
- [F/F2. Content sources (S1-S8)](#ff2-content-sources-s1-s8)
- [G/G2. Platform (X1-X20)](#gg2-platform-x1-x20)
- [F-series (Foliate)](#f-series-foliate)
- [How this pass changed the table](#how-this-pass-changed-the-table)
- [Top 15 gaps left (by value)](#top-15-gaps-left-by-value)
- [Sync umbrella (2026-11-20, syncui)](#sync-umbrella-2026-11-20-syncui)
- [Wave 11 (reader/text, T1/T2) 2026-11-21](#wave-11-readertext-t1t2-2026-11-21)

Statuses: **DONE** implemented, tested and reachable in the UI (or CLI where the row is CLI); **ENGINE ONLY** library code with tests, no UI; **PARTIAL**; **MISSING**; **DEFERRED** owner decision (PDF, Calibre wireless); **EXCLUDED** (LIT).

Effort S/M/L = rough size to finish the row. Priority H/M/L = my value ranking.

## Counts

Re-audited in full on 2026-11-22 (HEAD 805f4c2): every row that was PARTIAL or MISSING was re-checked against the tree (files, symbols, scenarios, `docs/*-notes.md`); DONE rows were kept unless a note showed a change. Not run on hardware, see `docs/remaining-work.md` Part 2 A.

| Status | 2026-11-13 | 2026-11-21 sample | 2026-11-22 full re-audit |
|---|---|---|---|
| DONE | 75 | 91 | **126** (R34 is REPLACED by per-book looks, counted apart) |
| ENGINE ONLY | 3 | 1 | **0** |
| PARTIAL | 64 | 58 | **46** |
| MISSING | 31 | 23 | **2** (C7 PDF output, P21 remote reader control) |
| DEFERRED | 3 | 3 | **3** (C4, P5, P13) |
| REPLACED | 0 | 0 | 1 (R34) |
| **Total rows** | 176 | 176 | 178 (R39b and the split C-rows are listed apart) |

Closed since 2026-11-13 (a PARTIAL or MISSING row became DONE): R3, R14, R22, R25, R32, R41, R43, R45, R49, R58, R59, R60, R65, C10, C20, P19, P20, S5, S6, S8, X10, X12, X19, F4 (and the 2026-11-21 sample rows). MISSING left: C7, P21.

| Section | DONE | PARTIAL | MISSING | DEFERRED | REPLACED |
|---|---|---|---|---|---|
| A. Reader (R1-R20, R65) | 17 | 4 | 0 | 0 | 0 |
| A2. Reader additions (R21-R64) | 27 | 17 | 0 | 0 | 1 |
| B. Library (L1-L13) | 12 | 1 | 0 | 0 | 0 |
| B2. Library additions (L14-L38) | 16 | 9 | 0 | 0 | 0 |
| D/D2. Conversion (C1-C21) | 11 | 3 | 1 | 1 | 0 |
| E. Devices and sharing (P1-P21) | 16 | 4 | 1 | 2 | 0 |
| F/F2. Content sources (S1-S8) | 7 | 1 | 0 | 0 | 0 |
| G/G2. Platform (X1-X20) | 14 | 6 | 0 | 0 | 0 |
| F-series (Foliate) | 6 | 1 | 0 | 0 | 0 |

## A. Reader (R1-R20, R65)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| R1 | Book CSS + user override layer | PARTIAL (2026-11-22 re-audit) | `bookx-text::css`, `bookx-core::html`/`doc`, `canvas.rs`, `tweaks.rs`; fonts and relative sizes from the book CSS plus per-book "book's own styling" / "book's fonts" switches (wave 11) | tables and positioned text from book CSS (tables still flatten to ` │ ` rows), hanging punctuation, `word-break`, emphasis dots, vertical tables and floats | L | M |
| R2 | Hyphenation | DONE | `bookx-text::hyphenate`, `canvas.rs` |  |  |  |
| R3 | Knuth-Plass, widows/orphans, drop caps, small caps | DONE (2026-11-22 re-audit) | `bookx-text::paragraph`, `canvas.rs`; `bookx-text::vertical::paginate_with` (widows/orphans 1 to 4, setting), `Style.small_caps` with Pango `font_scale` in `canvas.rs`/`doc.rs`/`html.rs` |  |  | L |
| R4 | Vertical writing, RTL, ruby | DONE (2026-11-21 refresh) | vertical-rl/lr, text-orientation, tate-chu-yoko, kinsoku, ruby, vertical selection and highlights in `canvas.rs`, `bookx-text::vertical`; scenarios `vertical_layout`, `vertical_chapters`, `vertical_scroll_and_styles`, `vertical_read_aloud`, session `vertical_reading`; `docs/archive/vertical-writing-plan.md` | hanging punctuation, `word-break`, emphasis dots, vertical tables and floats | L | M |
| R5 | Footnote popups | DONE (was MISSING) | `src/footnotes.rs`, `ui/reader_extra.rs`, reader `Input::Link`; scenario `footnote_popup` |  | S | H |
| R6 | Custom fonts per book / font manager | PARTIAL | `tweaks.rs` per-book font family | embedded fonts rendering, font manager | M | L |
| R7 | Highlight styles | DONE | `canvas.rs`, `reader.rs` |  |  |  |
| R8 | Dictionary lookup (StarDict) | DONE | `dictionaries.rs`, `bookx-lookup` | dictd, Wiktionary online | S | L |
| R9 | Vocabulary builder + flashcards | DONE | `vocabulary.rs`, `ui/vocab.rs` |  |  |  |
| R10 | Text-to-speech with sentence highlight | DONE | `bookx-tts`: Kokoro, Piper, voice server (speech-dispatcher removed in wave 4, enh. 20); player card (enh. 11); `ui/tts_settings.rs`, `ui/reader_tts.rs`, Save as audio (WAV, `tts_export.rs`), keyring API key, pause drain, chapter prefetch | MP3/OGG; the new pause and prefetch not heard on hardware |  |  |
| R11 | Reading statistics | DONE | `reading_stats.rs`, `goals.rs`, `ui/stats.rs` (goals, year calendar, review) |  | S | M |
| R12 | Book map / page browser | DONE (was ENGINE ONLY) | `bookx-lookup::bookmap`, `ui/reader_map.rs`, reader menu *Book Map*, palette; scenario `book_map` | highlight positions on the strip are estimates (no stored fraction); no page thumbnails | M | M |
| R13 | Tap zones, gestures, touch | DONE | `gestures.rs` | customisable gesture→action map (R36) | M | L |
| R14 | Night mode + warmth schedule, e-ink | DONE (2026-11-22 re-audit) | `eink.rs`, schemes; `bookx-text::aids::NightSchedule` (evening warm tint, dark theme), `src/aids_prefs.rs`, Preferences → Reading; e-ink in `eink.rs` |  |  | L |
| R15 | Status bar customisation | DONE (2026-11-13, enhancement 13) | `footer_stats.rs`, `ui/reader_footer.rs`: 15 items, order, presets, position, separator, size, opacity, per-book override; right click / Preferences → Reading → Footer | | | |
| R16 | Reading-position sync (KOSync) | DONE | `kosync_sync.rs`, `ui/kosync.rs`, `bookx-serve` |  |  |  |
| R17 | Fixed-layout / image books (CBZ, PDF) | PARTIAL (2026-11-22 re-audit) | CBZ/CBT/CBC in `bookx-formats` read in the comic reader (wave 6); PDF pages in the PDF reader; CBR through the `unrar`/`7z` helper (`bookx-formats::rar`, `comic`), DJVU through `ddjvu` (`bookx-formats::djvu`) | real RAR5 and DjVu files were never tried (helper subprocesses, tested with fixtures only); PDF has its own reader | L | M |
| R18 | Media overlays, embedded audio/video | PARTIAL (2026-11-22 re-audit) | `src/overlay_play.rs`, `src/narration.rs`: the reader plays a book's own EPUB 3 media-overlay narration (WAV) with the read-aloud card and highlight; the editor writes it (Add Narration, CE-422) | other audio codecs, embedded audio/video elements, narration made by other tools if not 16-bit PCM WAV | L | L |
| R19 | Per-book settings profiles | DONE (was PARTIAL) | `reading_profiles.rs`, `ui/reader_profiles.rs`: a profile can be the book's own (pin); scenario `reading_profiles` |  | S | M |
| R20 | Annotation export | DONE | `annotation_export.rs` (md, html, json, txt, Kindle clippings, Readwise payload, CSV); Anki (`bookx_lookup::anki`: .apkg, TSV, CSV, basic and cloze); Readwise upload (`readwise_sync.rs`) | Joplin, Obsidian templates; Anki / Readwise not tried against the real services | M | M |
| R65 | Page-turn animation | DONE (2026-11-22 re-audit) | `bookx-text::aids::PageAnimation`, `ui/canvas.rs` (slide and fade; off for reduced motion and e-ink), Preferences → Reading; scenario in `ui-tests-reader-ux.sh` | no page curl |  | L |

## A2. Reader additions (R21-R64)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| R21 | Style tweaks (CSS snippets, user css, per book) | PARTIAL | `tweaks.rs`, `ui/reader_style.rs` | curated snippet catalogue by category | M | L |
| R22 | Embedded styles / fonts toggles | DONE (2026-11-22 re-audit) | Preferences: use book's styling; per-book "book's own styling" and "book's fonts" switches (`ui/reader_style.rs`, `tweaks.rs`) |  |  | L |
| R23 | Text appearance controls | PARTIAL (2026-11-22 re-audit) | Preferences, Style Tweaks (size, line, margins, font, justify); weight, contrast, word spacing and kerning tweaks (`tweaks.rs`, `bookx-text::typography`) | hinting, page overlap, dpi | M | M |
| R24 | Page-numbering modes | PARTIAL | `bookx-text::pages`, `page_list.rs` (EPUB page-list to labels), footer shows *Page iv*, Go to… takes printed labels; scenario `reader_goto` | hidden flows; the page-list is read from the nav document only (NCX `pageList` is not) | M | M |
| R25 | Skim slider with preview | DONE (2026-11-22 re-audit) | progress slider; `bookx_text::aids::skim_chapter`, skim preview in `ui/reader.rs`/`reader_aids.rs` |  |  | L |
| R26 | Bookmarks, jump back/forward stack | DONE (was PARTIAL) | `jump_history.rs`, Back/Forward buttons, Alt+Left/Right, palette; scenario `reader_goto` |  | S | M |
| R27 | Book map and page browser | DONE (was ENGINE ONLY) | as R12 | page thumbnails (strip of rendered pages) not done | M | M |
| R28 | TOC tools (collapse, alt TOC, progress ticks) | PARTIAL (2026-11-22 re-audit) | sidebar TOC; contents folding (`src/toc_fold.rs`, `ui/reader_side.rs`) | alternative TOC, ticks on the progress bar | S | L |
| R29 | Highlight modes (extend, adjust after creation) | PARTIAL (2026-11-22 re-audit) | drag select, keyboard select; selection handles move a saved highlight (`ui/sel_handles.rs`, `src/selection_handles.rs`) | overlap picker, auto page-turn at the edge | M | L |
| R30 | Highlight colours/styles | DONE | 5 colours, 4 styles | legend/filter by colour (F4) | S | L |
| R31 | Editable highlight text, bookmark filters | PARTIAL (2026-11-22 re-audit) | Notes view filters by kind (highlights, notes, bookmarks) in `ui/notes.rs` | editing the highlighted text itself | S | L |
| R32 | Annotation export targets/services | DONE (2026-11-22 re-audit) | `annotation_export.rs`, `annotation_import.rs` (Kindle clippings, KOReader, Kobo, calibre), Readwise upload, Anki, Notes & Highlights window (`ui/notes.rs`); Joplin and Obsidian export added (`annotation_export.rs`) |  |  | M |
| R33 | Status bar items | DONE (2026-11-21 refresh; same work as R15) | `footer_stats.rs`, `ui/reader_footer.rs` (Footer Statistics dialog and Preferences group) |  |  |  |
| R34 | Reading profiles (named bundles, auto triggers) | REPLACED 2026-11-13 by automatic per-book looks (enh. 19) | `reading_profiles.rs` (`BookLooks`) - old: (Reading Profiles… window, export/import, per-book default) | auto triggers (time of day, language, device) | M | M |
| R35 | Quick menu | DONE (2026-11-21 refresh) | the command palette (`palette.rs`, Ctrl+Shift+P in the editor, library and reader) covers it |  |  |  |
| R36 | Gesture manager / keyboard map | PARTIAL | tap zones presets, swipe, keyboard map rebinding (`shortcuts.rs`, see L36) | user-defined gesture actions | M | M |
| R37 | Selection menu | DONE | popover: highlight, note, dictionary, Wikipedia (with the lead picture, enhancement 16), translate, copy; draggable selection handles (enhancement 10) | share, search-in-book from selection | S | L |
| R38 | Multiple dictionaries, lookup history, web templates | PARTIAL | `dictionaries.rs` | priority list UI, custom web lookup, history | S | L |
| R39 | Vocabulary: auto-add, due list, flashcards | DONE | `ui/vocab.rs` |  |  |  |
| R39b | Own vocabulary: terms with your own definition (no dictionary needed), marked in the text, Words list, glossary import/export, "My Vocabulary" dictionary | DONE (enhancements 32) | `bookx-lookup::{vocab,own,termmatch}`, `ui/vocab_*.rs`, `ui/reader_vocab.rs` | not in the catalogue or backups, no Ctrl+hover, no book-map entry | S | M |
| R40 | Statistics detail (sessions, estimates, calendar) | PARTIAL (2026-11-22 re-audit) | `ui/stats.rs` (year calendar via `goals::year_calendar`) | export of the statistics, per-book view | S | M |
| R41 | Search in book | DONE (2026-11-22 re-audit) | sidebar search, regex-less; whole-word / case / regex options (`ui/reader_search.rs`) |  |  | L |
| R42 | Reference mode (paper page numbers) | DONE (was MISSING) | `page_list.rs`, footer *Page N*, Go to… with printed labels; scenario `reader_goto` |  | S | L |
| R43 | Keyboard link following | DONE (2026-11-22 re-audit) | `bookx_text::aids::hint_labels`/`match_hint`, `ui/reader_aids.rs`; scenario `reader_link_hints` |  |  | L |
| R44 | Image zoom / viewer | DONE (was MISSING) | `ui/image_viewer.rs`, `image_view.rs`, canvas `on_image`; scenario `image_viewer` | inline images inside a line of text and pictures of fixed-layout books are not clickable | S | H |
| R45 | Copy with citation template | DONE (2026-11-22 re-audit) | `bookx_text::aids::cite`, Ctrl+Shift+C, template in Preferences → Reading (`ui/citations.rs`, `reader_aids.rs`) |  |  | L |
| R46 | Reading modes (scroll, spreads, max width) | PARTIAL | 1/2 column, continuous scroll | max-width setting, fixed-layout spreads | M | L |
| R47 | Themes: custom, per book, texture | PARTIAL | light/sepia/dark/black, Custom colours, theme editor with import/export (`reader_themes.rs`, `ui/reader_profiles.rs`); scenario `reader_themes` | per-book theme, background image/texture | M | M |
| R48 | RSVP speed reading | DONE (was ENGINE ONLY) | `rsvp_view.rs`, `ui/rsvp.rs`, reader menu and palette; scenario `speed_reading` | perception-expander lines | S | M |
| R49 | Auto-turn page timer | DONE (2026-11-22 re-audit) | `bookx_text::aids::AutoTurn`, Preferences → Reading, `ui/reader_aids.rs` |  |  | M |
| R50 | Read-aloud controls | DONE | `ui/reader_tts.rs` |  |  |  |
| R51 | Fixed-layout EPUB | PARTIAL | single page per spine item | spreads, positioned text | L | L |
| R52 | Comics/manga reader | PARTIAL (wave 6: reader DONE) | `crates/bookx-comic` + `ui/reader_comic.rs`: single, two-page and webtoon layouts, smart zoom, guided view, RTL per book, preloading with a memory budget, thumbnails, grey/e-ink, ComicInfo page types (`docs/comic-notes.md`) | next file in folder, real RAR, per-library direction, hardware e-ink check | L | M |
| R53 | Book information screen | DONE | reader Properties, library details panel |  |  |  |
| R54 | Custom cover per book | PARTIAL | cover replace in edit dialog | page screenshot as cover | S | L |
| R55 | Footnote popups configurable | DONE (was MISSING) | as R5; on/off in Preferences → Reading → Links | always/never only (no long-press mode) | S | H |
| R56 | Japanese: vertical, ruby | DONE (2026-11-21 refresh) | see R4; Aozora novel session `vertical_aozora` | dictionary extensions for Japanese | L | M |
| R57 | Per-book overrides (ignore CSS/metadata, reset data) | PARTIAL (2026-11-22 re-audit) | per-book "book's own styling" / "book's fonts" switches | library context entries (ignore metadata, reset data) | S | L |
| R58 | Move to archive after finishing | DONE (2026-11-22 re-audit) | Archive shelf for finished books (`aids_prefs::should_archive`, off by default) |  |  | L |
| R59 | Page-turn animation toggle, kinetic scroll | DONE (2026-11-22 re-audit) | as R65 (animation toggle); kinetic scroll not built |  |  | L |
| R60 | Warmth / auto night schedule | DONE (2026-11-22 re-audit) | as R14 (`NightSchedule`) |  |  | M |
| R61 | Remote HTTP control of the reader | PARTIAL (2026-11-22 re-audit) | Bluetooth page-turner keys (`bookx_text::aids::remote_map`, Preferences → Reading) | remote control over HTTP | M | L |
| R62 | Built-in text editor for sidecar/CSS | DONE | covered by Bookx editor |  |  |  |
| R63 | Reading goals, streaks, yearly review | DONE (was PARTIAL) | `goals.rs`, `ui/stats.rs`; scenario `reading_goals` |  | S | H |
| R64 | Accessibility: screen reader, high contrast, dyslexia fonts | PARTIAL (2026-11-22 re-audit) | `a11y.rs`, `ui/audit.rs`, OpenDyslexic option, e-ink high contrast, page text exposed to AT; **reading ruler / line focus (`reading_ruler.rs`, `ui/reading_ruler.rs`, Alt+Up/Down, Preferences → Reading; scenario `pain_ruler`, session `reading_ruler_keys`)**; reduced motion is honoured (`gtk-enable-animations` in `canvas.rs`) | image alt text read by AT; popover menu names unverified | M | M |

## B. Library (L1-L13)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| L1 | Metadata store (SQLite) | DONE | `bookx-store` |  |  |  |
| L2 | Library-wide full-text search | DONE | `fulltext.rs`, FTS5 |  |  |  |
| L3 | Custom columns | DONE (2026-11-21) | `bookx-core::custom` (types, validation, composites, undo), `bookx-store` schema 8 (`custom_columns`), `#label:` search, details/edit/bulk rows, table columns and sort, Calibre import/export, CLI `library columns`; `ui/custom_columns.rs` |  | L | H |
| L4 | Virtual libraries and saved searches | DONE (was MISSING) | `saved_searches.rs`, `ui/saved.rs`, browse panel; scenario `saved_searches` |  | M | H |
| L5 | Collections, shelves, reading status | DONE (was PARTIAL) | `shelves.rs`, `ui/shelves.rs`: Shelves in the browse panel, drag and drop, book menu, counts, order, smart shelves; scenario `shelves` | 'send shelf to device': the device window has a Kobo shelf name for sent books but no 'send this shelf' action, not wired; real drag and drop with the mouse is not covered by Broadway scenarios (the drop handler is) | M | H |
| L6 | Metadata download | DONE | `metadata_fetch.rs`, `bulk_fetch.rs` |  |  |  |
| L7 | Duplicate detection and merge | DONE | `duplicates.rs` |  |  |  |
| L8 | Series management | DONE | `series_view.rs`, `bookx-sources::tidy` |  |  |  |
| L9 | Library checks, backups, restore | DONE | `library_health.rs`, `bookx-backup` |  |  |  |
| L10 | Calibre import and export | DONE | `bookx-store::calibre`, `bookx-store::calibre_export` (wave 6: `export-calibre`, menu entry, verified with calibredb 9.15) | custom columns, reading state | M | L |
| L11 | Multiple libraries, switcher | DONE (was PARTIAL) | `libraries.rs`, `ui/library_switcher.rs`, header button; scenario `library_switcher` |  | S | M |
| L12 | Folder watcher | DONE (2026-11-21) | `ui/folder_watch.rs`: `gio::FileMonitor` on the import folder and its sub-folders plus a 60 s rescan, `InboxState.watch` (off by default), switch in Library Health; no `notify` crate |  | S | L |
| L13 | Cover tools | PARTIAL (2026-11-22 re-audit) | `bookx-sources::cover` generation/crop; edit dialog; **`cover_quality` grading, Health → Covers scan, larger covers offered as safe changes in the fetch review; `library covers`; scenario `pain_health`**; `src/library_cover.rs`: cover from the first page of a PDF, trim to 2:3 | interactive drag-to-crop dialog, side-by-side candidates | M | L |

## B2. Library additions (L14-L38)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| L14 | Add books: folders, archives, ISBN, empty record, add formats | PARTIAL (2026-11-21) | add by ISBN and empty record (`core::empty`, `ui/import_isbn.rs`), ZIP archives with several books (`core::archive_books`); RAR/7z archives and data files are not built | add by ISBN, empty record, archives with several books | M | M |
| L15 | Guess metadata from file names | DONE (2026-11-21) | `src/library_add.rs` applies `bookx-sources::filename` to new books; patterns editable in Preferences → Library |  | S | M |
| L16 | Bulk edit, search-replace, copy/paste metadata | PARTIAL (2026-11-21) | search and replace across fields with preview, regex and one undo step (`core::replace`, `ui/replace_fields.rs`, `library replace`); copy/paste of metadata and templates are not built | search-and-replace across fields, templates | M | M |
| L17 | Cover grid, bookshelf, cover flow | PARTIAL | grid and table, cover size | bookshelf/spines, flow | M | L |
| L18 | Tag browser | PARTIAL (2026-11-21) | tags as a tree (`src/library_tagtree.rs`, sidebar switch); drag and drop assignment and the not-filter are not built | hierarchical categories, drag-drop, not-filter | M | M |
| L19 | Quickview | DONE (2026-11-21) | Show Related Books (`src/library_related.rs`, `ui/library_t3.rs`) |  | S | L |
| L20 | Notes on authors/series/tags | PARTIAL (2026-11-21) | plain-text notes on authors and series (`bookx-store::notes`, schema 9) edited in the related books window; tags and publishers are in the store but have no editor, no rich text |  | M | L |
| L21 | Marked books | DONE (2026-11-21) | `Book::marked` (not saved), Edit → Mark or Unmark, `marked:true` |  | S | M |
| L22 | Search language (full Calibre syntax) | DONE (2026-11-21) | parentheses, `NOT`/`AND`, `~` regex, date fields with relative dates, `#label:`, `marked:`, `id:`, `vl:` (existing) |  | M | M |
| L23 | Virtual libraries combinable | DONE (was MISSING) | as L4: `search:`/`vl:` terms combine, `-search:` negation, virtual library narrows every view |  | S | H |
| L24 | Template language | PARTIAL | `bookx-sources::template` used for save paths/layout (`layout.rs`, Device window) | columns of type composite, display templates UI | M | M |
| L25 | Custom column types | DONE (2026-11-21) | see L3: text, long text, integer, float, rating, bool, date, tags, enumeration, composite; column rules and colouring are not built |  | M | H |
| L26 | Save to disk / send with templates | DONE | layout templates, device save-path templates, Save a Copy |  |  |  |
| L27 | Jobs panel | PARTIAL (2026-11-21) | sends from the Devices window and Add from the Web are jobs now; device cover sync is not | sends to devices, manual backup window, add from the web, device cover sync do not publish yet | M | M |
| L28 | Catalogs (CSV/BibTeX/EPUB catalogue) | PARTIAL (2026-11-21) | `catalog --format epub` (`core::catalog_epub`) and `#label` columns in csv/json; AZW3/MOBI through `convert`; no sections/prefix/exclusion options | EPUB/AZW3 catalogue | M | L |
| L29 | Collections/shelves, history, multi-select | DONE (was PARTIAL) | multi-select, shelves (see L5) |  | M | H |
| L30 | Reading status per book, finished date, ratings | DONE | status, rating in UI/search | review text | S | L |
| L31 | Sidecar import/export (KOReader) | PARTIAL | annotation + progress sync from KOReader sidecars | export reading state to sidecar | M | L |
| L32 | Library maintenance | DONE | Library Health window |  |  |  |
| L33 | Full-text index control | DONE (2026-11-21) | `ui/index_status.rs`: counts and Rebuild in Preferences → Library |  | S | L |
| L34 | Authors: sort, aliases, links | DONE | `authors_view.rs`, `ui/authors.rs` |  |  |  |
| L35 | Identifiers and links | DONE | details panel, cleanup |  |  |  |
| L36 | Keyboard shortcuts customisable | DONE (was MISSING) | `shortcuts.rs`, `ui/shortcuts_dialog.rs` (Preferences → Keyboard Shortcuts), live accelerators; scenarios `shortcuts_rebind`, `shortcuts_files`, `shortcuts_loaded`, session `shortcuts_real_keys` | the book editor's own keys and the selection keys (d w t h n) are fixed | M | H |
| L37 | Library undo | DONE | operation log + trash |  |  |  |
| L38 | Optimizing helper (warn about bad books) | DONE | Needs Attention, preflight |  |  |  |

## D/D2. Conversion (C1-C21)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| C1 | Pipeline core + CLI convert | DONE | `bookx-convert::pipeline`, `bookx-cli` |  |  |  |
| C2 | Inputs FB2, DOCX, ODT, RTF, CBZ… | DONE | `bookx-formats` (converter agent running) |  |  |  |
| C3 | MOBI/AZW3 read | DONE | `bookx-formats::mobi*` |  |  |  |
| C4 | PDF input | DEFERRED | `bookx-pdf` (PDF agent running) | reflow, OCR | XL | M |
| C5 | Outputs EPUB, KEPUB, TXT, MD, HTML, FB2, CBZ | DONE | `bookx-convert::writers` | CBZ writer: not a Calibre output |  |  |
| C6 | AZW3/MOBI write | DONE | `bookx-convert::mobi` | not tried on a real Kindle |  |  |
| C7 | PDF output | MISSING |  | writer | L | M |
| C8 | Heuristics | DONE | `bookx-convert::transforms` |  |  |  |
| C9 | Device profiles | DONE | `bookx-convert::options` |  |  |  |
| C10 | Batch convert queue + progress UI | DONE (2026-11-22 re-audit) | conversions run as jobs with progress and cancel (`jobs.rs`); `bookx-convert::queue` (at most N conversions at a time), jobs panel (`jobs.rs`) |  |  | M |
| C11-C16 | Look and feel, heuristics, page setup, metadata step, debug | DONE | `bookx-convert` transforms | a UI for the option groups beyond presets | M | M |
| C17 | Input formats Calibre accepts | PARTIAL (2026-11-22 re-audit) | all but DJVU, PDF (agent), AZW4 (wrapper), real RAR; LIT excluded; CBR (`unrar`/`7z`), DJVU (`ddjvu`), AZW4 (`azw4.rs`), LRF, CHM, TCR, PML, SNB, RB, PDB readers in `bookx-formats` | PDF reflow input is deferred (C4); real RAR/DjVu untested; LIT excluded | L | L |
| C18 | Output formats Calibre writes | PARTIAL (2026-11-22 re-audit) | EPUB, KEPUB, AZW3, MOBI, DOCX, FB2, HTMLZ, TXTZ, RTF, PDB, PMLZ, RB, SNB, OEB, ZIP | PDF, LRF | L | M |
| C19 | Comic input optimisation | PARTIAL (2026-11-22 re-audit) | CBZ read; CBZ/CBR read; comic converter options (`docs/comic-notes.md`) | device-optimised image processing (resize, grey, split spreads) on conversion | M | L |
| C20 | Per-format output options | DONE (2026-11-22 re-audit) | options struct + presets; `ui/convert_options.rs`, `src/convert_prefs.rs` (per-format options dialog) |  |  | L |
| C21 | Conversion presets | DONE | `bookx-convert` presets |  |  |  |

## E. Devices and sharing (P1-P21)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| P1 | Send to device (mass storage) | DONE | `devices.rs`, `ui/devices.rs` | never on real Kindle |  |  |
| P2 | Kobo KEPUB + DB shelf | DONE | `bookx-serve::kobo` | tested on built DBs only (a Kobo report exists) |  |  |
| P3 | KOSync server | DONE | `bookx-serve::kosync` |  |  |  |
| P4 | OPDS server | DONE | `bookx-serve::opds`, `ui/share.rs` |  |  |  |
| P4c | OPDS client (browse/search/download UI) | DONE (was MISSING) | `opds_browse.rs`, `ui/opds.rs`; scenarios `opds_browse`, `opds_errors` | not tried against the real Gutenberg/Standard Ebooks (no network in tests); basic auth only | M | H |
| P5 | Calibre wireless protocol | DEFERRED (owner decision D10, 2026-11-20: wireless sync is KOReader only; stock firmware = USB) | `docs/outdated/calibre-wireless-device-plan.md` | implementation (see plan) | L | L |
| P6 | Send by e-mail | DONE | `emailing.rs` |  |  |  |
| P7 | MTP | PARTIAL | gvfs mounts | direct USB MTP | L | L |
| P8 | Content server web UI | DONE | `bookx-serve::server`, `bookx-cli serve` |  |  |  |
| P9 | WebDAV/cloud folder | DONE | `bookx-serve::webdav`, devices, backups | only tested against a fake server |  |  |
| P10 | Content server: accounts, permissions, PWA | PARTIAL (2026-11-22 re-audit) | one user/password, HTTPS; per-user accounts and guest collections (`bookx-serve::accounts`, CLI only), trusted proxy, allowed hosts and sub-path prefix (`ui/proxy_settings.rs`), PWA web client (`pwa.rs`) | accounts screen in Preferences (settings file and CLI only), PWA not run in a real browser | L | M |
| P11 | OPDS search incl. content, pagination | DONE | `bookx-serve::opds` |  |  |  |
| P12 | KOSync endpoints | DONE |  |  |  |  |
| P13 | Calibre-plugin interop (KOReader) | DEFERRED | see P5 |  | M | L |
| P13b | KOReader highlight/note/bookmark sync over Wi-Fi (own plugin + built-in server) | DONE (not run on a device) | `crates/bookx-serve/src/annotation_sync.rs`, `koreader-plugin/bookx.koplugin/`, `src/annotation_sync*.rs`, wiki `koreader-sync.md`, `docs/koreader-annotation-sync.md`; scenarios `koreader_annotations*` | the plugin menu was never run inside KOReader; Bookx highlights are listed on the device, not painted; bookmarks are not imported | M | M |
| P14 | Device drivers (Kobo, Kindle, PocketBook, Boox…) | PARTIAL | generic + Kobo + Kindle detection, reading state from Kobo/KOReader | Kindle native reading state | L | L |
| P15 | Send options (format fallback, templates, remove, sync back) | DONE | Device window |  |  |  |
| P16 | Share: email, folder, LAN | DONE | Share Library, e-mail |  |  |  |
| P17 | Remote file browser, cloud sync | PARTIAL | WebDAV device | FTP/SSH | M | L |
| P18 | OPDS client with presets, auth | DONE (was MISSING) | as P4c: presets Gutenberg and Standard Ebooks, add catalogue with auth (password in keyring), search, format preference |  | M | H |
| P19 | Wallabag / read-it-later sync | DONE (2026-11-22 re-audit) | read-later is local only; `bookx-sources::wallabag`, `ui/wallabag.rs` (Add from the Web), scenario `wallabag_import`; tried against a loopback stub only |  |  | L |
| P20 | QR code share of library URL | DONE (2026-11-22 re-audit) | `bookx-serve::qr` (own encoder), `ui/qr_dialog.rs` (Sync → QR code); scenario `sync_qr_code` |  |  | L |
| P21 | Remote reader control (R61) | MISSING |  |  | S | L |

## F/F2. Content sources (S1-S8)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| S1 | Read-later (web article to EPUB) | DONE | `web_import.rs`, Add from the Web | queue/list UI of saved articles | S | L |
| S2 | News/RSS to EPUB | DONE | `news.rs`, `ui/news.rs` |  |  |  |
| S3 | Public-domain catalogues via OPDS | DONE (was MISSING) | as P4c |  | S | H |
| S4 | News recipes (declarative, scheduling) | PARTIAL (2026-11-22 re-audit) | feeds + schedule + keep-N; recipe model in `bookx-sources::recipe` | recipe editor UI (cleanup rules), per-recipe accounts for paywalled feeds (not started, needs a design for where the secret lives) | M | M |
| S5 | RSS downloader per-feed settings | DONE (2026-11-22 re-audit) | OPML, per-feed switch; `FeedSpec::full_article`, `news::split_feed_line` (`url full|summary`) |  |  | L |
| S6 | Metadata sources config and priorities | DONE (2026-11-22 re-audit) | `metadata_settings.rs`, merge by confidence; `source_priority.rs`, `ui/source_settings.rs` (per-field priority), name-order toggle; scenario `sources_priority_ui` | Hardcover client has no screen |  | L |
| S7 | Metadata read/write to files | DONE | `bookx-core::opf`, edit writes into EPUB | AZW3/PDF write-back | S | L |
| S8 | Wikipedia article as book | DONE (2026-11-22 re-audit) | inline summary only; `bookx-sources::wikipedia`, `wikipedia:Title` in Add from the Web (`web_import.rs`) |  |  | L |

## G/G2. Platform (X1-X20)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| X1 | Workspace split | DONE | 13 crates |  |  |  |
| X2 | CLI | DONE | `bookx-cli` |  |  |  |
| X3 | WASM plugins | DONE | `bookx-plugin`, Preferences |  |  |  |
| X4 | Performance harness | DONE | `perf_budgets.rs` in 6 crates | reader/scroll budgets partly via scenario `reader_budget` |  |  |
| X5 | Adaptive layout for phones | PARTIAL | narrow-window overlays in library | reader/prefs on phone sizes | M | M |
| X6 | Localization | PARTIAL | `i18n.rs` (`tr`, `tr_c`, `trf`, `ntr`, `N_`, plural rules, number/date helpers), `po/` (pot, es, de complete; fr, hi 191 strings), `build-aux/i18n.sh`, ~1,600 strings marked across `src/ui` and the glue modules by script; tests keep template, es/de, placeholders and plural forms in step; wiki *Translating Bookx* | es/de/fr/hi are machine-assisted and need native review; roughly 700 literals are still English (enum label tables, composed device-sync sentences, export labels, markup fragments, error texts); no Weblate project | M | H |
| X7 | Flatpak and AppImage, update notice | PARTIAL | the Flatpak builds offline (`build-aux/flatpak.sh`, GNOME 50, espeak-ng module, cargo sources script) and its smoke tests pass; release/CI workflows; `make release-check`; see `docs/packaging-notes.md` | AppImage, update notice, Flathub submission; devices/voices/secrets not exercised in the sandbox | S | L |
| X8 | Plugin API | DONE | = X3 |  |  |  |
| X9 | User patches | DONE | `bookx-plugin::patch` |  |  |  |
| X10 | Profiles/gestures/quick menu as config; import/export settings | DONE (2026-11-22 re-audit) | `src/settings_io.rs`, `ui/settings_io.rs` (Export/Import Settings in Preferences) |  |  | M |
| X11 | Single searchable Preferences page | DONE | `ui/preferences.rs` with search and registry |  |  |  |
| X12 | CLI tool coverage | DONE (2026-11-22 re-audit) | convert, meta, check, library, search, serve; `bookx-cli`: `catalog`, `fetch-news`, `polish`, `fonts`, `library columns`, `library replace`, `library extras`, `backup`, `highlights`, `mcp`, `plugin` |  |  | M |
| X13 | E-ink mode | DONE | `eink.rs` | not on hardware |  |  |
| X14 | Keyboard-first + command palette | DONE | `palette.rs`, `ui/palette.rs` |  |  |  |
| X15 | Offline-first principles | DONE | no telemetry; **offline mode switch and the list of connections (`privacy.rs`, Preferences → Privacy, wiki Privacy; tests in `bookx-sources/tests/offline.rs`, `bookx-tts/tests/offline.rs`, scenario `pain_offline`)** |  |  |  |
| X16 | Release channels, deb/rpm | PARTIAL | Flatpak bundle, tarball (deterministic packing), `.deb`/`.rpm` metadata and jobs, `SHA256SUMS`, `CHANGELOG.md`, `release-check.sh` (workflow never run on GitHub; `cargo deb`/`generate-rpm` never run here) | binary reproducibility, signing, install tests on other distros | M | L |
| X17 | Localization workflow | PARTIAL | as X6; RTL default direction for right-to-left languages (unit tested, no RTL catalogue ships); locale-aware numbers, plurals and some dates | live re-translation of open windows; Weblate project | M | H |
| X18 | Performance harness extended | PARTIAL | budgets for open, search, import, conversion | scroll fps, memory ceiling | M | L |
| X19 | Diagnostics: system info, debug log export, safe mode | DONE (2026-11-22 re-audit) | `src/diagnostics.rs` with a unit-tested `redact`; About → Troubleshooting and *Copy Diagnostics* (About dialog, palette, `app.copy-diagnostics`); wiki Troubleshooting; `src/safe_mode.rs` (safe mode, `--safe-mode`), `src/debug_log.rs` (debug log export), `src/diagnostics.rs` |  |  | M |
| X20 | `--doc`/`bookx://` links, MIME registration for all formats | DONE (links forwarded only via dev command) | `src/links.rs`, `app.open-uri`, second-process forwarding, desktop file with 16 media types + `x-scheme-handler/bookx`, `data/*.mime.xml`, `desktop_integration` tests, scenarios `uri_links` | `--doc`; a real launcher test | S | M |

## F-series (Foliate)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| F1 | Reader sidebar tabs | DONE (redesigned 2026-11-13, enhancement 12 / bug 9) | book header (cover, title, author, Properties), Contents / Bookmarks / Notes as virtualised lists, notes with search + colour/style/note filters and per-highlight actions; `ui/reader_side.rs`, `side_notes.rs` |  |  |  |
| F2 | Quick note | DONE |  |  |  |  |
| F3 | Cross-reference preview | DONE (was MISSING) | as R5 |  | S | H |
| F4 | Colour legend/filter of highlights | DONE (2026-11-22 re-audit) | colour legend and filter in the Notes window (`ui/notes.rs`) |  |  | L |
| F5 | Jump history | DONE (was PARTIAL) | = R26 |  | S | M |
| F6 | Bookmark shortcut, last position bookmark | PARTIAL | Ctrl+B; position remembered | automatic bookmark | S | L |
| F7 | Image zoom/pan | DONE (was MISSING) | = R44 |  | S | H |

## How this pass changed the table

Implemented in this pass (each with unit tests, a UI scenario and a wiki page): L4/L23 saved searches and virtual
libraries, P4c/P18/S3 online (OPDS) catalogues, R5/R55/F3 footnote popups, R48 speed reading, R63/R11 reading goals,
year calendar and yearly review, X6/X17 translation infrastructure with Spanish and German samples (partial).
Other agents finished, during the same days: TTS engines (R10), vertical writing and furigana (R4, ruby done), the
metadata leftovers (formats of one book, author aliases, DRM detection, trackers, audiobooks), backup destinations
with the keyring, the Preferences sidebar, the command palette, accessibility, the device UI, the KOSync client,
annotation import/export, and the editor rows (Check Book, search/function mode, spelling, reports, special
characters; see `docs/archive/calibre-editor-gap-analysis.md`). Those were already DONE in the table above, so they do not
move the counts.

## Top 15 gaps left (by value)

1. ~~**L3/L25 custom columns**~~ done 2026-11-21 (wave 11); column colouring rules remain.
2. ~~**L5/L29 collections and shelves UI**~~ done 2026-11-13 (ux-gaps).
3. ~~**L36 / R36 keyboard shortcut customisation**~~ done 2026-11-13 (ux-gaps); R36 gesture actions remain.
4. ~~**R44/F7 image viewer**~~ done 2026-11-13 (ux-gaps).
5. ~~**R26/F5 jump history forward stack, go to page/percent dialog.**~~ done 2026-11-13 (ux-gaps).
6. ~~**R34/R19 reading profiles and R47 theme editor**~~ done 2026-11-13 (ux-gaps); profile auto triggers, per-book theme, background texture remain.
7. ~~**R12/R27 book map**~~ done 2026-11-13 (ux-gaps); page thumbnails remain.
8. ~~**R20/R32 Readwise upload and Anki export**~~ done 2026-11-13 (annot-extras), see `docs/annotation-sync-notes.md`.
9. **X6 finish translations**: the remaining ~700 English literals (see `docs/packaging-notes.md`), native review of es/de/fr/hi, Weblate project.
10. ~~**L27/C10 jobs panel**~~ done 2026-11-13 (ux-gaps) for imports, conversions, mail, fetches, scheduled backups and news; device sends, the backup window and web import still to publish.
11. ~~**L11 library switcher**~~ done 2026-11-13 (ux-gaps).
12. **S4 news recipe editor UI** (cleanup rules, accounts).
13. **P10 content server accounts and permissions**, reverse-proxy prefix.
14. ~~**R24/R42 real page numbers from `page-list`**~~ done 2026-11-13 (ux-gaps); hidden flows and NCX `pageList` remain.
15. ~~**X19 debug-log export and safe mode.**~~ done in wave 11 (`safe_mode.rs`, `debug_log.rs`).

Runners-up (R49, R60, R33, R65, L14 ISBN/empty record, L21 are done; what is left): P5 Calibre wireless (plan only, see `docs/outdated/calibre-wireless-device-plan.md`; judged larger
than a day), C7 PDF output, C4/R17 PDF (PDF agent).

## Sync umbrella (2026-11-20, syncui)

| Item | Status | Where |
|---|---|---|
| One umbrella “Sync”: main menu entry (was Share Library), one Preferences → Sync page (was Devices and Sharing + Sync), old name kept as an alias | DONE | `src/sync_model.rs`, `src/ui/sync_prefs.rs`, `src/ui/sync_window.rs`, `src/ui/library.rs` |
| Task-oriented Sync window: status card (off / ready / connected) with one main button, three choice cards (Bookx sync with KOReader, USB device, Share books), collapsed Advanced | DONE | `src/ui/sync_window.rs` |
| Preferences → Sync: four-step setup checklist (server, account with Register / Log in inline, plugin copy, test on the device), What Bookx sync does, USB, library servers, collapsed Advanced holding every technical setting and the Requirements table | DONE | `src/ui/sync_prefs.rs`, `src/ui/share.rs`, `src/ui/kosync.rs`, `src/annotation_sync_ui.rs` |
| One settings bus for both surfaces; no duplicated switch with its own state | DONE | `src/settings_bus.rs` (`sync()`), `share::follow` |
| “KOSync” removed from labels, titles and buttons (named once as “Bookx sync (extended KOSync)”); no `/kosync` text; wiki pages renamed and rewritten | DONE | wiki `progress-sync.md`, `koreader-sync.md`, `devices-and-sharing.md`, `preferences.md`, `reverse-proxy.md` |
| Research note, before/after IA, where each old setting went | DONE | `docs/sync-ux-notes.md` |
| Not done: “connected” only reflects annotation sync; a hand-copied plugin is not detected; not tried on a real Kobo or KOReader device | caveat | `docs/sync-ux-notes.md` |

## Wave 11 (reader/text, T1/T2) 2026-11-21

Done: R3 widows/orphans setting (`vertical::paginate_with`, 1 to 4), real small caps (`Style.small_caps`, Pango `font_scale`),
R23 weight / contrast / word spacing / kerning tweaks, R1 fonts and relative sizes from the book CSS plus a per-book
"book's own styling" and "book's fonts" switch (R57 part), R65/R59 slide and fade page-turn animation (off for reduced
motion and e-ink), R49 auto-turn, R60 evening schedule (warm tint, dark theme), R25 skim preview, R28 contents folding,
R29 handles move a saved highlight, R43 link hints, R45 copy with citation, R58 Archive shelf (off by default), R61 page
turner keys, whole-word / case / regex in-book search, Joplin export. Logic in `bookx-text::aids`, `src/aids_prefs.rs`,
`src/toc_fold.rs`; UI in `src/ui/reader_aids.rs` (child of `reader.rs`) and `reader_settings_extra.rs`.

Already existed: R12/R27 page thumbnails (`ui/map_thumbs.rs`), F4 colour legend and filter (`ui/notes.rs`), Obsidian export.
Not done: tables and positioned text from book CSS (tables still flatten to ` │ ` rows), hanging punctuation, `word-break`,
emphasis dots, vertical tables/floats, R31 editing the highlight text, R57 library context entries, R38/R40/R41 UI (the engine
in `bookx-lookup::dictset` exists), stats calendar export, PDF book map, R46/R51, R18. Page-turn animation is a slide-away /
fade of the old page over the new one (no page curl). Unverified: real Bluetooth page turner, real-session input.
