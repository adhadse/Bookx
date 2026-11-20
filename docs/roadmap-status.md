# Roadmap status (audit of the code, not the progress log)

Audited 2026-11-13 against `docs/bookx-feature-roadmap.md` Parts 1 and 3 (E-series rows are in `docs/calibre-editor-gap-analysis.md`). Status was taken from the code, the tests, the wiki and the UI scenarios, not from the progress log. Rows whose area another agent was working on that day say so.

Statuses: **DONE** implemented, tested and reachable in the UI (or CLI where the row is CLI); **ENGINE ONLY** library code with tests, no UI; **PARTIAL**; **MISSING**; **DEFERRED** owner decision (PDF, Calibre wireless); **EXCLUDED** (LIT).

Effort S/M/L = rough size to finish the row. Priority H/M/L = my value ranking.

## Counts

| Status | Before this pass | After |
|---|---|---|
| DONE | 65 | 75 (87 after the ux-gaps pass) |
| ENGINE ONLY | 4 | 3 (1) |
| PARTIAL | 63 | 64 (59) |
| MISSING | 41 | 31 (26) |
| DEFERRED | 3 | 3 |
| EXCLUDED | 0 | 0 |
| **Total rows** | 176 | 176 |

| Section | DONE | ENGINE ONLY | PARTIAL | MISSING | DEFERRED | EXCLUDED |
|---|---|---|---|---|---|---|
| A. Reader (R1-R20, R65) | 12 | 0 | 7 | 2 | 0 | 0 |
| A2. Reader additions (R21-R64) | 14 | 0 | 19 | 11 | 0 | 0 |
| B. Library (L1-L13) | 10 | 0 | 2 | 1 | 0 | 0 |
| B2. Library additions (L14-L38) | 10 | 1 | 10 | 4 | 0 | 0 |
| D/D2. Conversion (C1-C21) | 9 | 0 | 5 | 1 | 1 | 0 |
| E. Devices and sharing (P1-P21) | 13 | 0 | 4 | 3 | 2 | 0 |
| F/F2. Content sources (S1-S8) | 4 | 0 | 3 | 1 | 0 | 0 |
| G/G2. Platform (X1-X20) | 10 | 0 | 8 | 2 | 0 | 0 |
| F-series (Foliate) | 5 | 0 | 1 | 1 | 0 | 0 |

## A. Reader (R1-R20, R65)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| R1 | Book CSS + user override layer | PARTIAL | `bookx-text::css`, `bookx-core::html`/`doc`, `canvas.rs`, `tweaks.rs` | indents, margins, floats, drop caps, italic/bold etc. done; colours/fonts/tables/positioning from book CSS only via Original colours | L | M |
| R2 | Hyphenation | DONE | `bookx-text::hyphenate`, `canvas.rs` |  |  |  |
| R3 | Knuth-Plass, widows/orphans, drop caps, small caps | PARTIAL | `bookx-text::paragraph`, `canvas.rs` | widows/orphans control, small-caps synthesis | S | L |
| R4 | Vertical writing, RTL, ruby | PARTIAL | vertical-rl/lr, RTL, furigana in `canvas.rs`, `bookx-text::vertical` (vertical agent) | tate-chu-yoko edge cases; selection in vertical | L | M |
| R5 | Footnote popups | DONE (was MISSING) | `src/footnotes.rs`, `ui/reader_extra.rs`, reader `Input::Link`; scenario `footnote_popup` |  | S | H |
| R6 | Custom fonts per book / font manager | PARTIAL | `tweaks.rs` per-book font family | embedded fonts rendering, font manager | M | L |
| R7 | Highlight styles | DONE | `canvas.rs`, `reader.rs` |  |  |  |
| R8 | Dictionary lookup (StarDict) | DONE | `dictionaries.rs`, `bookx-lookup` | dictd, Wiktionary online | S | L |
| R9 | Vocabulary builder + flashcards | DONE | `vocabulary.rs`, `ui/vocab.rs` |  |  |  |
| R10 | Text-to-speech with sentence highlight | DONE | `bookx-tts`: Kokoro, Piper, voice server (speech-dispatcher removed in wave 4, enh. 20); player card (enh. 11); `ui/tts_settings.rs`, `ui/reader_tts.rs`, Save as audio (WAV, `tts_export.rs`), keyring API key, pause drain, chapter prefetch | MP3/OGG; the new pause and prefetch not heard on hardware |  |  |
| R11 | Reading statistics | DONE | `reading_stats.rs`, `goals.rs`, `ui/stats.rs` (goals, year calendar, review) |  | S | M |
| R12 | Book map / page browser | DONE (was ENGINE ONLY) | `bookx-lookup::bookmap`, `ui/reader_map.rs`, reader menu *Book Map*, palette; scenario `book_map` | highlight positions on the strip are estimates (no stored fraction); no page thumbnails | M | M |
| R13 | Tap zones, gestures, touch | DONE | `gestures.rs` | customisable gesture→action map (R36) | M | L |
| R14 | Night mode + warmth schedule, e-ink | PARTIAL | `eink.rs`, schemes | warmth/auto-dark schedule (R60) | S | L |
| R15 | Status bar customisation | DONE (2026-11-13, enhancement 13) | `footer_stats.rs`, `ui/reader_footer.rs`: 15 items, order, presets, position, separator, size, opacity, per-book override; right click / Preferences → Reading → Footer | | | |
| R16 | Reading-position sync (KOSync) | DONE | `kosync_sync.rs`, `ui/kosync.rs`, `bookx-serve` |  |  |  |
| R17 | Fixed-layout / image books (CBZ, PDF) | PARTIAL | CBZ/CBT/CBC in `bookx-formats` read in the comic reader (wave 6); PDF pages in the PDF reader | CBR real RAR | L | M |
| R18 | Media overlays, embedded audio/video | MISSING |  | SMIL playback | L | L |
| R19 | Per-book settings profiles | DONE (was PARTIAL) | `reading_profiles.rs`, `ui/reader_profiles.rs`: a profile can be the book's own (pin); scenario `reading_profiles` |  | S | M |
| R20 | Annotation export | DONE | `annotation_export.rs` (md, html, json, txt, Kindle clippings, Readwise payload, CSV); Anki (`bookx_lookup::anki`: .apkg, TSV, CSV, basic and cloze); Readwise upload (`readwise_sync.rs`) | Joplin, Obsidian templates; Anki / Readwise not tried against the real services | M | M |
| R65 | Page-turn animation | MISSING |  | animated page curl/slide, setting | L | L |

## A2. Reader additions (R21-R64)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| R21 | Style tweaks (CSS snippets, user css, per book) | PARTIAL | `tweaks.rs`, `ui/reader_style.rs` | curated snippet catalogue by category | M | L |
| R22 | Embedded styles / fonts toggles | PARTIAL | Preferences: use book's styling | embedded fonts toggle | S | L |
| R23 | Text appearance controls | PARTIAL | Preferences, Style Tweaks (size, line, margins, font, justify) | weight, contrast, word spacing, kerning, hinting, page overlap, dpi | M | M |
| R24 | Page-numbering modes | PARTIAL | `bookx-text::pages`, `page_list.rs` (EPUB page-list to labels), footer shows *Page iv*, Go to… takes printed labels; scenario `reader_goto` | hidden flows; the page-list is read from the nav document only (NCX `pageList` is not) | M | M |
| R25 | Skim slider with preview | PARTIAL | progress slider | chapter preview popover | S | L |
| R26 | Bookmarks, jump back/forward stack | DONE (was PARTIAL) | `jump_history.rs`, Back/Forward buttons, Alt+Left/Right, palette; scenario `reader_goto` |  | S | M |
| R27 | Book map and page browser | DONE (was ENGINE ONLY) | as R12 | page thumbnails (strip of rendered pages) not done | M | M |
| R28 | TOC tools (collapse, alt TOC, progress ticks) | PARTIAL | sidebar TOC | collapse/expand, ticks on progress bar | S | L |
| R29 | Highlight modes (extend, adjust after creation) | PARTIAL | drag select, keyboard select | adjust handles, overlap picker, auto page-turn at edge | M | L |
| R30 | Highlight colours/styles | DONE | 5 colours, 4 styles | legend/filter by colour (F4) | S | L |
| R31 | Editable highlight text, bookmark filters | MISSING |  | UI | S | L |
| R32 | Annotation export targets/services | PARTIAL | `annotation_export.rs`, `annotation_import.rs` (Kindle clippings, KOReader, Kobo, calibre), Readwise upload, Anki, Notes & Highlights window (`ui/notes.rs`) | Joplin | M | M |
| R33 | Status bar items | MISSING |  | configurable items | M | L |
| R34 | Reading profiles (named bundles, auto triggers) | REPLACED 2026-11-13 by automatic per-book looks (enh. 19) | `reading_profiles.rs` (`BookLooks`) - old: (Reading Profiles… window, export/import, per-book default) | auto triggers (time of day, language, device) | M | M |
| R35 | Quick menu | MISSING | command palette covers most use |  | S | L |
| R36 | Gesture manager / keyboard map | PARTIAL | tap zones presets, swipe, keyboard map rebinding (`shortcuts.rs`, see L36) | user-defined gesture actions | M | M |
| R37 | Selection menu | DONE | popover: highlight, note, dictionary, Wikipedia (with the lead picture, enhancement 16), translate, copy; draggable selection handles (enhancement 10) | share, search-in-book from selection | S | L |
| R38 | Multiple dictionaries, lookup history, web templates | PARTIAL | `dictionaries.rs` | priority list UI, custom web lookup, history | S | L |
| R39 | Vocabulary: auto-add, due list, flashcards | DONE | `ui/vocab.rs` |  |  |  |
| R39b | Own vocabulary: terms with your own definition (no dictionary needed), marked in the text, Words list, glossary import/export, "My Vocabulary" dictionary | DONE (enhancements 32) | `bookx-lookup::{vocab,own,termmatch}`, `ui/vocab_*.rs`, `ui/reader_vocab.rs` | not in the catalogue or backups, no Ctrl+hover, no book-map entry | S | M |
| R40 | Statistics detail (sessions, estimates, calendar) | PARTIAL | `ui/stats.rs` | calendar, export, per-book view | S | M |
| R41 | Search in book | DONE | sidebar search, regex-less | whole word / case / regex options | S | L |
| R42 | Reference mode (paper page numbers) | DONE (was MISSING) | `page_list.rs`, footer *Page N*, Go to… with printed labels; scenario `reader_goto` |  | S | L |
| R43 | Keyboard link following | MISSING |  | link hints | S | L |
| R44 | Image zoom / viewer | DONE (was MISSING) | `ui/image_viewer.rs`, `image_view.rs`, canvas `on_image`; scenario `image_viewer` | inline images inside a line of text and pictures of fixed-layout books are not clickable | S | H |
| R45 | Copy with citation template | MISSING |  |  | S | L |
| R46 | Reading modes (scroll, spreads, max width) | PARTIAL | 1/2 column, continuous scroll | max-width setting, fixed-layout spreads | M | L |
| R47 | Themes: custom, per book, texture | PARTIAL | light/sepia/dark/black, Custom colours, theme editor with import/export (`reader_themes.rs`, `ui/reader_profiles.rs`); scenario `reader_themes` | per-book theme, background image/texture | M | M |
| R48 | RSVP speed reading | DONE (was ENGINE ONLY) | `rsvp_view.rs`, `ui/rsvp.rs`, reader menu and palette; scenario `speed_reading` | perception-expander lines | S | M |
| R49 | Auto-turn page timer | MISSING |  |  | S | M |
| R50 | Read-aloud controls | DONE | `ui/reader_tts.rs` |  |  |  |
| R51 | Fixed-layout EPUB | PARTIAL | single page per spine item | spreads, positioned text | L | L |
| R52 | Comics/manga reader | PARTIAL (wave 6: reader DONE) | `crates/bookx-comic` + `ui/reader_comic.rs`: single, two-page and webtoon layouts, smart zoom, guided view, RTL per book, preloading with a memory budget, thumbnails, grey/e-ink, ComicInfo page types (`docs/comic-notes.md`) | next file in folder, real RAR, per-library direction, hardware e-ink check | L | M |
| R53 | Book information screen | DONE | reader Properties, library details panel |  |  |  |
| R54 | Custom cover per book | PARTIAL | cover replace in edit dialog | page screenshot as cover | S | L |
| R55 | Footnote popups configurable | DONE (was MISSING) | as R5; on/off in Preferences → Reading → Links | always/never only (no long-press mode) | S | H |
| R56 | Japanese: vertical, ruby | PARTIAL | vertical agent running | ruby, vertical selection | L | M |
| R57 | Per-book overrides (ignore CSS/metadata, reset data) | MISSING |  | library context entries | S | L |
| R58 | Move to archive after finishing | MISSING |  |  | S | L |
| R59 | Page-turn animation toggle, kinetic scroll | MISSING |  |  | S | L |
| R60 | Warmth / auto night schedule | MISSING |  | time-based scheme switch | S | M |
| R61 | Remote HTTP control of the reader | MISSING |  |  | M | L |
| R62 | Built-in text editor for sidecar/CSS | DONE | covered by Bookx editor |  |  |  |
| R63 | Reading goals, streaks, yearly review | DONE (was PARTIAL) | `goals.rs`, `ui/stats.rs`; scenario `reading_goals` |  | S | H |
| R64 | Accessibility: screen reader, high contrast, dyslexia fonts | PARTIAL | `a11y.rs`, `ui/audit.rs`, OpenDyslexic option, e-ink high contrast, page text exposed to AT; **reading ruler / line focus (`reading_ruler.rs`, `ui/reading_ruler.rs`, Alt+Up/Down, Preferences → Reading; scenario `pain_ruler`, session `reading_ruler_keys`)** | reduced motion, image alt text | M | M |

## B. Library (L1-L13)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| L1 | Metadata store (SQLite) | DONE | `bookx-store` |  |  |  |
| L2 | Library-wide full-text search | DONE | `fulltext.rs`, FTS5 |  |  |  |
| L3 | Custom columns | MISSING | no column definitions in the catalogue | definitions + editing + display + search `#col:` (touches store: metadata agent area) | L | H |
| L4 | Virtual libraries and saved searches | DONE (was MISSING) | `saved_searches.rs`, `ui/saved.rs`, browse panel; scenario `saved_searches` |  | M | H |
| L5 | Collections, shelves, reading status | DONE (was PARTIAL) | `shelves.rs`, `ui/shelves.rs`: Shelves in the browse panel, drag and drop, book menu, counts, order, smart shelves; scenario `shelves` | 'send shelf to device': the device window has a Kobo shelf name for sent books but no 'send this shelf' action, not wired; real drag and drop with the mouse is not covered by Broadway scenarios (the drop handler is) | M | H |
| L6 | Metadata download | DONE | `metadata_fetch.rs`, `bulk_fetch.rs` |  |  |  |
| L7 | Duplicate detection and merge | DONE | `duplicates.rs` |  |  |  |
| L8 | Series management | DONE | `series_view.rs`, `bookx-sources::tidy` |  |  |  |
| L9 | Library checks, backups, restore | DONE | `library_health.rs`, `bookx-backup` |  |  |  |
| L10 | Calibre import and export | DONE | `bookx-store::calibre`, `bookx-store::calibre_export` (wave 6: `export-calibre`, menu entry, verified with calibredb 9.15) | custom columns, reading state | M | L |
| L11 | Multiple libraries, switcher | DONE (was PARTIAL) | `libraries.rs`, `ui/library_switcher.rs`, header button; scenario `library_switcher` |  | S | M |
| L12 | Folder watcher | PARTIAL | inbox (rescan on open/start) | background `notify` watcher | S | L |
| L13 | Cover tools | PARTIAL (2026-11-20) | `bookx-sources::cover` generation/crop; edit dialog; **`cover_quality` grading, Health → Covers scan, larger covers offered as safe changes in the fetch review; `library covers`; scenario `pain_health`** | crop UI, extract from PDF, side-by-side candidates | M | L |

## B2. Library additions (L14-L38)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| L14 | Add books: folders, archives, ISBN, empty record, add formats | PARTIAL | files, folders, drag and drop, add format to a book (`ui/formats.rs`), editions view | add by ISBN, empty record, archives with several books | M | M |
| L15 | Guess metadata from file names | ENGINE ONLY | `bookx-sources::filename` | import option UI | S | M |
| L16 | Bulk edit, search-replace, copy/paste metadata | PARTIAL | bulk edit dialog, tidy names | search-and-replace across fields, templates | M | M |
| L17 | Cover grid, bookshelf, cover flow | PARTIAL | grid and table, cover size | bookshelf/spines, flow | M | L |
| L18 | Tag browser | PARTIAL | browse panel with counts | hierarchical categories, drag-drop, not-filter | M | M |
| L19 | Quickview | MISSING |  |  | S | L |
| L20 | Notes on authors/series/tags | MISSING |  |  | M | L |
| L21 | Marked books | MISSING |  |  | S | M |
| L22 | Search language (full Calibre syntax) | PARTIAL | `bookx-core::query`: fields, =, comparisons, OR, -, text:, weak: | dates/relative, regex, `#custom:`, `marked:`, parentheses | M | M |
| L23 | Virtual libraries combinable | DONE (was MISSING) | as L4: `search:`/`vl:` terms combine, `-search:` negation, virtual library narrows every view |  | S | H |
| L24 | Template language | PARTIAL | `bookx-sources::template` used for save paths/layout (`layout.rs`, Device window) | columns of type composite, display templates UI | M | M |
| L25 | Custom column types | MISSING | see L3 |  | M | H |
| L26 | Save to disk / send with templates | DONE | layout templates, device save-path templates, Save a Copy |  |  |  |
| L27 | Jobs panel | PARTIAL | `jobs.rs`, `ui/jobs.rs`: imports, conversions, e-mailing, metadata fetches, scheduled backups, news; cancel for imports, conversions, fetches; scenario `jobs_panel` | sends to devices, manual backup window, add from the web, device cover sync do not publish yet | M | M |
| L28 | Catalogs (CSV/BibTeX/EPUB catalogue) | PARTIAL (2026-11-20) | Goodreads/StoryGraph CSV export; **Export Citations (BibTeX, RIS, CSL JSON, APA): `bookx-sources::citation`, `ui/citations.rs`, `library cite`; scenario `pain_citations`** | EPUB/AZW3 catalogue | M | L |
| L29 | Collections/shelves, history, multi-select | DONE (was PARTIAL) | multi-select, shelves (see L5) |  | M | H |
| L30 | Reading status per book, finished date, ratings | DONE | status, rating in UI/search | review text | S | L |
| L31 | Sidecar import/export (KOReader) | PARTIAL | annotation + progress sync from KOReader sidecars | export reading state to sidecar | M | L |
| L32 | Library maintenance | DONE | Library Health window |  |  |  |
| L33 | Full-text index control | PARTIAL | auto background indexing | index status/rebuild UI | S | L |
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
| C10 | Batch convert queue + progress UI | PARTIAL | conversions run as jobs with progress and cancel (`jobs.rs`) | no queue limiting parallel conversions | M | M |
| C11-C16 | Look and feel, heuristics, page setup, metadata step, debug | DONE | `bookx-convert` transforms | a UI for the option groups beyond presets | M | M |
| C17 | Input formats Calibre accepts | PARTIAL | all but DJVU, PDF (agent), AZW4 (wrapper), real RAR; LIT excluded |  | L | L |
| C18 | Output formats Calibre writes | PARTIAL | EPUB, KEPUB, AZW3, MOBI, DOCX, FB2, HTMLZ, TXTZ, RTF, PDB, PMLZ, RB, SNB, OEB, ZIP | PDF, LRF | L | M |
| C19 | Comic input optimisation | PARTIAL | CBZ read | device-optimised image processing | M | L |
| C20 | Per-format output options | PARTIAL | options struct + presets | UI | M | L |
| C21 | Conversion presets | DONE | `bookx-convert` presets |  |  |  |

## E. Devices and sharing (P1-P21)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| P1 | Send to device (mass storage) | DONE | `devices.rs`, `ui/devices.rs` | never on real Kindle |  |  |
| P2 | Kobo KEPUB + DB shelf | DONE | `bookx-serve::kobo` | tested on built DBs only (a Kobo report exists) |  |  |
| P3 | KOSync server | DONE | `bookx-serve::kosync` |  |  |  |
| P4 | OPDS server | DONE | `bookx-serve::opds`, `ui/share.rs` |  |  |  |
| P4c | OPDS client (browse/search/download UI) | DONE (was MISSING) | `opds_browse.rs`, `ui/opds.rs`; scenarios `opds_browse`, `opds_errors` | not tried against the real Gutenberg/Standard Ebooks (no network in tests); basic auth only | M | H |
| P5 | Calibre wireless protocol | DEFERRED (owner decision D10, 2026-11-20: wireless sync is KOReader only; stock firmware = USB) | `docs/calibre-wireless-device-plan.md` | implementation (see plan) | L | L |
| P6 | Send by e-mail | DONE | `emailing.rs` |  |  |  |
| P7 | MTP | PARTIAL | gvfs mounts | direct USB MTP | L | L |
| P8 | Content server web UI | DONE | `bookx-serve::server`, `bookx-cli serve` |  |  |  |
| P9 | WebDAV/cloud folder | DONE | `bookx-serve::webdav`, devices, backups | only tested against a fake server |  |  |
| P10 | Content server: accounts, permissions, PWA | PARTIAL | one user/password, HTTPS | multiple users with permissions per library, reverse-proxy prefix, PWA | L | M |
| P11 | OPDS search incl. content, pagination | DONE | `bookx-serve::opds` |  |  |  |
| P12 | KOSync endpoints | DONE |  |  |  |  |
| P13 | Calibre-plugin interop (KOReader) | DEFERRED | see P5 |  | M | L |
| P13b | KOReader highlight/note/bookmark sync over Wi-Fi (own plugin + built-in server) | DONE (not run on a device) | `crates/bookx-serve/src/annotation_sync.rs`, `koreader-plugin/bookx.koplugin/`, `src/annotation_sync*.rs`, wiki `koreader-sync.md`, `docs/koreader-annotation-sync.md`; scenarios `koreader_annotations*` | the plugin menu was never run inside KOReader; Bookx highlights are listed on the device, not painted; bookmarks are not imported | M | M |
| P14 | Device drivers (Kobo, Kindle, PocketBook, Boox…) | PARTIAL | generic + Kobo + Kindle detection, reading state from Kobo/KOReader | Kindle native reading state | L | L |
| P15 | Send options (format fallback, templates, remove, sync back) | DONE | Device window |  |  |  |
| P16 | Share: email, folder, LAN | DONE | Share Library, e-mail |  |  |  |
| P17 | Remote file browser, cloud sync | PARTIAL | WebDAV device | FTP/SSH | M | L |
| P18 | OPDS client with presets, auth | DONE (was MISSING) | as P4c: presets Gutenberg and Standard Ebooks, add catalogue with auth (password in keyring), search, format preference |  | M | H |
| P19 | Wallabag / read-it-later sync | MISSING | read-later is local only | Wallabag API | M | L |
| P20 | QR code share of library URL | MISSING |  |  | S | L |
| P21 | Remote reader control (R61) | MISSING |  |  | S | L |

## F/F2. Content sources (S1-S8)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| S1 | Read-later (web article to EPUB) | DONE | `web_import.rs`, Add from the Web | queue/list UI of saved articles | S | L |
| S2 | News/RSS to EPUB | DONE | `news.rs`, `ui/news.rs` |  |  |  |
| S3 | Public-domain catalogues via OPDS | DONE (was MISSING) | as P4c |  | S | H |
| S4 | News recipes (declarative, scheduling) | PARTIAL | feeds + schedule + keep-N; recipe model in `bookx-sources::recipe` | recipe editor UI (cleanup rules), per-recipe accounts | M | M |
| S5 | RSS downloader per-feed settings | PARTIAL | OPML, per-feed switch | per-feed full-article option UI | S | L |
| S6 | Metadata sources config and priorities | PARTIAL | `metadata_settings.rs`, merge by confidence | per-field priority UI, more sources | M | L |
| S7 | Metadata read/write to files | DONE | `bookx-core::opf`, edit writes into EPUB | AZW3/PDF write-back | S | L |
| S8 | Wikipedia article as book | MISSING | inline summary only |  | S | L |

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
| X10 | Profiles/gestures/quick menu as config; import/export settings | MISSING |  |  | S | M |
| X11 | Single searchable Preferences page | DONE | `ui/preferences.rs` with search and registry |  |  |  |
| X12 | CLI tool coverage | PARTIAL | convert, meta, check, library, search, serve | catalog, fetch-news, polish, embed/subset-fonts, saved_searches, custom_columns | M | M |
| X13 | E-ink mode | DONE | `eink.rs` | not on hardware |  |  |
| X14 | Keyboard-first + command palette | DONE | `palette.rs`, `ui/palette.rs` |  |  |  |
| X15 | Offline-first principles | DONE | no telemetry; **offline mode switch and the list of connections (`privacy.rs`, Preferences → Privacy, wiki Privacy; tests in `bookx-sources/tests/offline.rs`, `bookx-tts/tests/offline.rs`, scenario `pain_offline`)** |  |  |  |
| X16 | Release channels, deb/rpm | PARTIAL | Flatpak bundle, tarball (deterministic packing), `.deb`/`.rpm` metadata and jobs, `SHA256SUMS`, `CHANGELOG.md`, `release-check.sh` (workflow never run on GitHub; `cargo deb`/`generate-rpm` never run here) | binary reproducibility, signing, install tests on other distros | M | L |
| X17 | Localization workflow | PARTIAL | as X6; RTL default direction for right-to-left languages (unit tested, no RTL catalogue ships); locale-aware numbers, plurals and some dates | live re-translation of open windows; Weblate project | M | H |
| X18 | Performance harness extended | PARTIAL | budgets for open, search, import, conversion | scroll fps, memory ceiling | M | L |
| X19 | Diagnostics: system info, debug log export, safe mode | PARTIAL (was MISSING) | `src/diagnostics.rs` with a unit-tested `redact`; About → Troubleshooting and *Copy Diagnostics* (main menu, palette, `app.copy-diagnostics`); wiki Troubleshooting | debug log export, safe mode | S | M |
| X20 | `--doc`/`bookx://` links, MIME registration for all formats | DONE (links forwarded only via dev command) | `src/links.rs`, `app.open-uri`, second-process forwarding, desktop file with 16 media types + `x-scheme-handler/bookx`, `data/*.mime.xml`, `desktop_integration` tests, scenarios `uri_links` | `--doc`; a real launcher test | S | M |

## F-series (Foliate)

| ID | Feature | Status | Where in the code | What is missing | Effort | Prio |
|---|---|---|---|---|---|---|
| F1 | Reader sidebar tabs | DONE (redesigned 2026-11-13, enhancement 12 / bug 9) | book header (cover, title, author, Properties), Contents / Bookmarks / Notes as virtualised lists, notes with search + colour/style/note filters and per-highlight actions; `ui/reader_side.rs`, `side_notes.rs` |  |  |  |
| F2 | Quick note | DONE |  |  |  |  |
| F3 | Cross-reference preview | DONE (was MISSING) | as R5 |  | S | H |
| F4 | Colour legend/filter of highlights | MISSING |  |  | S | L |
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
characters; see `docs/calibre-editor-gap-analysis.md`). Those were already DONE in the table above, so they do not
move the counts.

## Top 15 gaps left (by value)

1. **L3/L25 custom columns** (definitions, editing, display, `#col:` search, composite template columns). Touches the store schema, so it belongs with the catalogue/metadata work.
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
15. **X19 debug-log export and safe mode.** (System information, MIME registration and `bookx://` links are done.)

Runners-up: R49 auto-turn, R60 warmth schedule, R33 status bar items, R65 page-turn animation, L14 add by ISBN / empty
record, L21 marked books, P5 Calibre wireless (plan only, see `docs/calibre-wireless-device-plan.md`; judged larger
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
