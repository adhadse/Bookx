# Bookx Feature Roadmap

Audit trail document. Part 1 is the original draft (v0.1), kept verbatim. Part 2 records the owner's decisions (D1-D10) and what they change. Part 3 is the extended appendix built from the official KOReader user guide and wiki index, the Calibre manual, and the Foliate/KOReader/Calibre READMEs. Agents: read Part 2 before Part 1, because Part 2 overrides the draft where they differ.

Date written: 2026-11-12. Status: see PART 4 (progress log) at the end of this file. Parts 1-3 are the plan and are not edited when work lands; Part 4 is.

---

# PART 1: Original draft v0.1 (verbatim)

# Bookx Feature Roadmap — Draft v0.1

## 1. The Bookx flavour

KOReader is fast and tweakable but looks like a device menu. Calibre does everything, but slowly and cluttered. Foliate reads beautifully but does little else. Bookx's pitch is **Calibre's power, KOReader's typesetting depth and Foliate's calm, in one native Rust binary that is fast and quiet.**

Four ideas I'd build the identity around:

1. **Instant by contract.** Performance budgets that tests enforce:
   - open a book in under 100 ms;
   - scroll 10,000 covers at 60 fps;
   - full-text search across the whole library in under 50 ms;
   - import 1,000 books in under 60 s using all cores.
2. **Speaks everyone's protocols.** Built-in OPDS server, KOSync-compatible progress server and Calibre wireless-device protocol, so KOReader, Kobo, Kindle and phones plug straight in. No other reader does this.
3. **A commonplace book, not a reader.** Highlights and notes are first-class: a cross-book notes view, export to Markdown, Obsidian, Readwise or Anki.
4. **One engine, three faces.** A `bookx` CLI and headless server, the GTK app, and a library crate. Each ships as one static Rust binary.

## 2. Feature catalogue

Each row has an ID so you can reply "R1 yes, C4 no". Status is relative to Bookx today. Effort is S, M, L or XL.

### A. Reader (KOReader and Foliate)

| ID  | Feature | From | Status | Effort | Notes |
|-----|---|---|---|---|---|
| R1  | Apply the book's own CSS (subset) with a user override layer | KOR, Fol | missing | XL | Biggest fidelity gap. Use `lightningcss` or `cssparser`, then map onto the canvas |
| R2  | Hyphenation, multilingual (Liang patterns) | KOR, Fol | missing | M | `hypher` crate, pure Rust |
| R3  | Better line breaking (Knuth-Plass), widows/orphans, drop caps, small caps | new | partial | L | Part of the "best typography on Linux" pitch |
| R4  | Vertical writing, RTL, ruby | Fol | DONE (2026-11-13, see Part 4) | L | Needed for CJK and Arabic. Pango supports it |
| R5  | Footnote popups | Fol, KOR | missing | S | Preview of the link target inline |
| R6  | Custom fonts per book, font manager, embedded font support | KOR | partial | M | `fontdb` |
| R7  | Highlight styles (background, underline, strikethrough, squiggly), tap-to-edit | KOR | partial | S | Only background exists |
| R8  | Dictionary lookup: StarDict/dictd, Wiktionary, offline first | KOR, Fol | missing | M | Wikipedia and translate already exist |
| R9  | Vocabulary builder, flashcards (spaced repetition) | KOR | missing | M | Ties into the commonplace-book idea |
| R10 | Text-to-speech with sentence highlight | Fol | missing | M | Only non-Rust piece: system speech-dispatcher. Flag as an exception |
| R11 | Reading statistics: time, pages per day, streaks, goals, heatmap | KOR | missing | M | Local-first, charts in the library |
| R12 | Book map / page browser / skim mode (thumbnails of chapters) | KOR | missing | M | |
| R13 | Tap zones, gestures, touch and tablet mode, customizable actions | KOR | partial | M | Adaptive libadwaita layout |
| R14 | Night mode with warmth schedule, e-ink mode (no animation, high contrast) | KOR | missing | S | |
| R15 | Status bar customization (clock, battery, chapter marks on the progress bar) | KOR | partial | S | |
| R16 | Reading-position sync across devices (KOSync protocol) | KOR | missing | M | See P3 |
| R17 | Fixed-layout and image books: CBZ, CBR, PDF (continuous, reflow) | KOR, Cal | CBZ DONE; PDF DONE (wave 4, `bookx-pdf`) | XL | Page view: continuous/single, zoom, crop, dark pages, thumbnails, outline, labels, links, search, selection + highlights/notes, password, position; reflow experimental. Not done: selection across pages, OCG, forms. `docs/pdf-research.md` |
| R18 | Media overlays (audio-synced EPUB), embedded audio and video | Fol | missing | L | Low priority |
| R19 | Per-book settings profiles; "typography presets" | KOR | missing | S | |
| R20 | Annotation export (JSON, Markdown, HTML, Readwise, Obsidian, Anki) | KOR, Fol | partial | S | JSON, Markdown and text exist |
| R65 | Ereader simulating animation when turning pages, which kind of looks like you're actually interracting with a ereader when turning the page | independent | missing | L | Low priority |

### B. Library and metadata (Calibre)

| ID | Feature | Status | Effort | Notes |
|---|---|---|---|---|
| L1 | Metadata store beyond JSON: embedded DB (`redb`) with indexes, migrations, Calibre `metadata.db` import | partial | L | JSON won't hold 100k books. Decision D2 on SQLite vs pure-Rust |
| L2 | Library-wide full-text search (`tantivy`) | missing | M | Headline performance feature |
| L3 | Custom columns (text, number, rating, date, bool, series, enumeration) | missing | M | |
| L4 | Virtual libraries and saved searches | missing | S | The search language already exists |
| L5 | Collections, shelves, reading status (to-read, on hold, finished) | partial | S | KOReader-style status |
| L6 | Metadata download: Open Library, Google Books, Wikidata, ISBN lookup, cover search | missing | M | Plugin trait for sources |
| L7 | Duplicate detection and merge books | partial | M | Only a naive title and author check today |
| L8 | Series management, auto-numbering | partial | S | |
| L9 | Library checks, backups, restore from a folder | missing | S | |
| L10 | Calibre library import/migration (read `metadata.db`) and export back | done (import; export wave 6) | L | Fastest on-ramp for Calibre users |
| L11 | Multiple libraries, switcher | missing | S | |
| L12 | Parallel, incremental folder watcher ("drop books in a folder") | missing | S | `notify` crate |
| L13 | Cover tools: generate cover, crop, extract from PDF | missing | M | |

### C. Editing and polishing (Calibre editor, new work)

| ID | Feature | Status | Effort | Notes |
|---|---|---|---|---|
| E1 | "Polish book": subset embedded fonts, smarten punctuation, remove unused CSS, update metadata in-place | missing | L | `subsetter` for fonts |
| E2 | Link checker with auto-repair, rename-with-refactor (done for files), move files | partial | M | |
| E3 | Reports: unused CSS, image sizes, word frequency, character stats | missing | M | |
| E4 | Compare books / diff two EPUBs | missing | M | |
| E5 | Spell check, grammar hints in editor (offline) | missing | M | `spellbook` (Hunspell-compatible, pure Rust) |
| E6 | Syntax: CSS and XML auto-complete, tag matching, bracket matching, snippets | missing | M | |
| E7 | Image tools: compress/convert in place (`image`, `oxipng`), crop | missing | M | |
| E8 | EPUB 3 accessibility checker and EPUB validation (spec-based rules) | partial | L | Our check is a subset |
| E9 | Split and merge chapters, split at headings, merge CSS | missing | M | |
| E10 | Visual TOC from preview (click heading → entry) | partial | S | |

### D. Conversion and formats (Calibre)

Proposed architecture: **Input → Doc IR (we have one) → transforms → Output**, parallelised per chapter with `rayon`.

| ID | Item | Effort | Notes |
|---|---|---|---|
| C1 | Conversion pipeline core: traits, transform stages, progress and cancellation, CLI `bookx convert` | L | The foundation for the rest of this table |
| C2 | Inputs: EPUB (done), TXT/MD/HTML (done), FB2, DOCX, ODT, RTF, CBZ | M each | `quick-xml`, `docx-rs` |
| C3 | Inputs: MOBI/AZW3/KF8 (read) | L | `mobi` crate exists. KF8 needs more work |
| C4 | Inputs: PDF (text reflow, optional OCR hooks) | XL | DONE (wave 4): `bookx-formats/src/pdf.rs` reflows text (columns, header/footer removal, hyphen joining, chapters from outline), metadata, cover from page 1; scans/protected files import with a note. Experimental. OCR: design note only (`docs/pdf-research.md`) |
| C5 | Outputs: EPUB 2/3 (clean), KEPUB (Kobo), TXT, MD, HTML/HTMLZ, FB2, CBZ | M each | KEPUB is just spans and a container, so cheap and useful |
| C6 | Outputs: AZW3/KF8, MOBI | XL | Closed spec. High risk. Decision D4 |
| C7 | Outputs: PDF with page setup, headers/footers, margins | L (not done; not required in wave 4) | `krilla` or `printpdf` + our layout engine, which reuses the canvas |
| C8 | Heuristics: structure detection, auto-TOC, chapter splitting, line-unwrap, smart quotes | L | Calibre's "look and feel" |
| C9 | Device profiles (Kindle, Kobo, generic e-ink): image resize, margins, fonts | M | |
| C10 | Batch convert queue with parallel jobs and a progress UI | M | Shares the JOBS-style config |

### E. Devices and sharing (Calibre, KOReader)

| ID | Feature | Effort | Notes |
|---|---|---|---|
| P1 | Send to device over USB mass storage (Kobo, Kindle, Boox, generic): detect mount, copy, keep folders | M | Pure `gio` and file copy |
| P2 | Kobo: KEPUB conversion on send, write to its DB (needs SQLite, see D2) | L | |
| P3 | KOSync-compatible progress server, built in | M | Lets KOReader users sync with Bookx |
| P4 | OPDS server (browse and download from any reader app) and OPDS client (add catalogs) | M | `axum` |
| P5 | Calibre wireless-device protocol (smart device app) | L | Documented, but large |
| P6 | Send by email (Kindle) | S | `lettre` |
| P7 | MTP for Android phones | L | Needs `libmtp` unless a pure-Rust crate matures |
| P8 | Content server web UI (read and download in a browser) | L | Builds on P4 and a headless mode |
| P9 | Cloud folders: WebDAV, Dropbox-style via WebDAV only | M | KOReader parity |

### F. Content sources (KOReader and Calibre)

| ID | Feature | Effort | Notes |
|---|---|---|---|
| S1 | Read-later: save a web article as EPUB (readability extraction) | M | Wallabag-style, `readability` + `feed-rs` |
| S2 | News/RSS to EPUB "recipes" on a schedule | L | Calibre's headline feature. Use Rhai or WASM for recipes, not Python |
| S3 | Browse and download from public-domain catalogs (Gutenberg, Standard Ebooks) via OPDS | S | Free once P4 client exists |

### G. Platform and extensibility

| ID | Feature | Effort | Notes |
|---|---|---|---|
| X1 | Workspace split: `bookx-core`, `bookx-format-*`, `bookx-convert`, `bookx-store`, `bookx-ui`, `bookx-cli` | L | Prerequisite for parallel work |
| X2 | `bookx` CLI: `convert`, `meta`, `check`, `serve`, `import`, `search`, JSON output | M | Replaces `ebook-convert`, `calibredb`, `ebook-meta` |
| X3 | WASM plugin system (`wasmtime`) with a small stable ABI: metadata sources, converters, news recipes | XL | Decision D5 |
| X4 | Performance harness: criterion benches and budgets as tests | M | Makes "blazing fast" checkable |
| X5 | Adaptive layout for phones and tablets (Phosh, touch targets) | M | |
| X6 | Localization (gettext-compatible, Weblate-friendly) | M | |
| X7 | Flatpak and AppImage releases, auto-update notice | S | |

## 3. Licence guardrails

- **KOReader is AGPL-3.0.** I would not copy any of its code. Reimplement behaviour from user docs and protocol specs, which is fine.
- **Calibre and Foliate are GPL-3.0.** Bookx is GPL-3.0-or-later, so code could be borrowed in principle. Calibre is Python and Foliate is JavaScript, so there is little to reuse. I propose clean-room reimplementation in Rust throughout.
- Any asset copied from them (icons, dictionaries, hyphenation patterns) goes in `NOTICE.md`.

## 4. Parallel execution plan for multiple agents

Parallel agents collide when they share files, so the plan starts with a single-agent foundation wave that sets the boundaries.

**Wave 0 — Foundation (one agent, a few days):**
- X1 workspace split with stable trait contracts (`FormatReader`, `FormatWriter`, `Transform`, `MetadataSource`, `DeviceDriver`, `Store`).
- X4 performance harness.
- `AGENTS.md` extended with the crate ownership map.

**Wave 1 — Independent tracks, each in its own crate, with its own tests:**

| Track | Scope | Owns |
|---|---|---|
| T1 Typography | R1–R7, R19 | canvas, new `bookx-layout` |
| T2 Dictionary and learning | R8, R9, R11, R12 | `bookx-lookup`, `bookx-stats` |
| T3 Store and search | L1–L5, L7, L9, L11, L12 | `bookx-store` |
| T4 Formats in | C2, C3, R17 (CBZ), C4 | `bookx-format-*` |
| T5 Conversion | C1, C5, C7–C10, X2 | `bookx-convert`, CLI |
| T6 Devices and servers | P1–P6, P8 | `bookx-devices`, `bookx-serve` |
| T7 Editor polish | E1–E10 | editor and core editing |
| T8 Metadata and sources | L6, L10, L13, S1–S3 | `bookx-sources` |
| T9 UX and platform | R13–R15, X5, X6 | UI crates |
| T10 Plugins | X3 | `bookx-plugin` |

**Wave 2 — Integration:** one agent merges the tracks, resolves UI wiring, runs the full performance and UI suites, and builds the release.

Each track delivers:
- unit tests;
- a UI scenario where it touches the UI;
- a performance budget test;
- a docs page.

That matches the rules already in `AGENTS.md`. Conflicts are avoided by file ownership per crate and by trait-only interfaces between crates.

## 5. Decisions I need from you

| # | Question | My recommendation |
|---|---|---|
| D1 | Which feature IDs are in, out, or later? | Start with R1, R2, R5, R7, R8, R11, L1, L2, L10, C1, C5, P1, P3, P4, X1, X2, X4 |
| D2 | Pure-Rust-only store (`redb`, no SQLite), or allow SQLite via `rusqlite`? Kobo support and Calibre `metadata.db` import need SQLite reading | Allow SQLite only for import and Kobo, and use `redb` for our own store |
| D3 | PDF: build on young pure-Rust renderers (`hayro`), or defer and keep PDF out of scope? | Defer. CBZ yes, PDF later |
| D4 | AZW3/MOBI writing is a closed, high-risk format. Include? | Read only. Write KEPUB and EPUB instead, since modern Kindles take EPUB |
| D5 | Plugin system now (WASM) or after the core ships? | After Wave 2 |
| D6 | Is TTS acceptable despite needing speech-dispatcher? | Yes, optional |
| D7 | Should the CLI and headless server be a first-class product? | Yes, since it is Calibre's second strength |
| D8 | E-ink and tablet modes: real targets or nice-to-have? | Nice-to-have, after R13 |
| D9 | Any Calibre or KOReader feature you want explicitly excluded? | You decide |

## 6. Suggested first release milestone ("Bookx 0.5")

Beyond this, the "unique flavour" bundle would be R1, R2, R5, R7, R8, R11, L2, C1/C5 (EPUB↔KEPUB, FB2, DOCX in), P1, P3, P4, X2 and X4. It gives you real typography, a stats page, a library you can search in milliseconds, sending to Kobo and KOReader, and a CLI. Together those make Bookx visibly different from all three source projects.

One caveat: the KOReader and Calibre feature lists come from my memory, not from reading the wikis. Before any track starts, I'd have each agent verify its slice against the official docs.

---

# PART 2: Owner decisions (authoritative, override Part 1)

| # | Decision | Effect on the plan |
|---|---|---|
| D1 | **Everything goes in.** | No feature is excluded. Order is set by waves and tracks, not by cutting. |
| D2 | **Use SQLite via `rusqlite`.** | SQLite is allowed everywhere (own store, Calibre `metadata.db` import/export, Kobo DB). Part 1's idea of `redb` as the main store is dropped unless benchmarks show a need. Keep the `Store` trait so the backend stays swappable. |
| D3 | **PDF deferred. CBZ in.** **Status (wave 4): PDF DONE** (read, reflow, annotations, search, import) on pure-Rust `hayro`; OCR still open (design note). | R17: CBZ now (and CBR/CBT if cheap). PDF read (C4, PDF reflow, PDF highlights, OCR) was scheduled after Wave 2 and shipped in wave 4. |
| D4 | **If Calibre supports a format, Bookx should too.** Write independent, high-quality tests, **only for formats Calibre actually does.** | Overrides draft: **AZW3/KF8 and MOBI writing are in scope (C6)**. Each format needs its own round-trip and conformance tests, written independently of the implementation (golden files, third-party validators where available, e.g. epubveri for EPUB; Calibre's own `ebook-meta`/`ebook-convert` as an oracle in optional CI jobs). Do not add test coverage or output formats Calibre does not have. |
| D5 | **Plugin system after Wave 2.** | T10 (X3, WASM plugins) starts only after Wave 2 integration. Until then keep the trait contracts plugin-ready but internal. |
| D6 | **TTS: yes.** | R10 in scope (speech-dispatcher is the accepted non-Rust exception). |
| D7 | **CLI and headless server: YES, first-class.** | X2, P4, P8 are first-class products, not afterthoughts. CLI must reach parity with `ebook-convert`, `ebook-meta`, `calibredb`, `calibre-server` basics. |
| D8 | **E-ink/tablet mode: nice to have, but must come after R13.** | R13 (gestures/touch/tablet) first, then e-ink mode (R14 e-ink part, new items K-series below). |
| D9 | **No exclusions.** | Everything in Part 3 is a candidate. **Amended 2026-11-12 (owner): LIT (Microsoft Reader) is excluded** because Microsoft discontinued the format. Bookx only recognises a `.lit` file and rejects it with a clear message; no decoder is to be written. |
| D10 | **Wireless sync is supported only for KOReader** (any device that runs it). No wireless sync for stock Kobo/Kindle firmware or other devices; they use USB. (2026-11-20) | Calibre wireless protocol (P5/P13) stays deferred. Highlights and notes sync over Wi-Fi through the Bookx plugin for KOReader and the built-in server (`docs/koreader-annotation-sync.md`, wiki "KOReader highlights and notes"). |

Standing rules from the project (`AGENTS.md`): every change ships with tests; UI tests run in parallel, concurrency configurable (`JOBS`, `RUST_TEST_THREADS`); Rust-only application code; no debug output in committed code.

Provisional wave order after decisions:
- Wave 0: X1 workspace split, X4 perf harness, trait contracts (incl. `Store` on rusqlite), AGENTS.md ownership map.
- Wave 1: tracks T1-T9 in parallel (T4 includes CBZ, T5 includes AZW3/MOBI writers with independent tests).
- Wave 2: integration.
- After Wave 2: T10 plugins (X3), PDF (C4/R17-pdf: DONE in wave 4), e-ink polish beyond R13.

---

# PART 3: Extended appendix (researched from the official docs)

Sources actually read:
- KOReader user guide (koreader.rocks/user_guide, version 26.11): full table of contents and these sections in full or in part: 2 basic usage, 3 status bar/menus, 4 navigation, 5 formatting/style tweaks, 6 annotating/exporting highlights, 9 library (file browser, collections, optimizing, OPDS, progress sync, metadata), 10 gestures, 11 quick menu/profiles, 12 plugins list, 13 faster reading, 14 tips. Plus KOReader README and wiki index (112 pages).
- Calibre manual pages (heading level, plus selected action lists): GUI, E-book viewer, Editing e-books, Content server, News/recipes, Catalogs, Virtual libraries, conversion overview; calibre "About" and README.
- Foliate README (the features list is from the project README and my knowledge of the app, not a source dump).

Not verified from source (pages returned 404 placeholders or were not read in detail): Calibre column creation tutorial, full-text search page, conversion option reference, metadata sources page, sync page; most individual KOReader wiki pages (Statistics, Vocabulary builder, Progress sync protocol, Dictionary, Hyphenation, Cover browser, user patches). Before each track starts, its agent must read the relevant manual/wiki page and correct the item below.

Naming: new IDs continue the Part 1 series. "KOR" = KOReader, "CAL" = Calibre, "FOL" = Foliate. Effort S/M/L/XL as before. Licence rule from Part 1 section 3 applies: behaviour only, no code copying from KOReader.

## A2. Reader additions (KOReader, Calibre viewer, Foliate)

| ID | Feature | From | Effort | Notes |
|---|---|---|---|---|
| R21 | Style tweaks: curated, toggleable CSS snippets by category, plus user `.css` tweak files, plus per-book tweak editor | KOR 5.3 | M | Maps onto R1 override layer. Examples: heading separators, paragraph gaps, hide images, tidy code blocks |
| R22 | Embedded styles and embedded fonts on/off toggles | KOR | S | Part of R1 |
| R23 | Text appearance controls: font size, font weight, contrast, line spacing, word spacing and word expansion (justification), kerning mode, hinting, margins (L/R/top/bottom), page overlap, zoom (dpi), render mode | KOR 2.3, 3.5, 5 | M | Typography panel with live preview |
| R24 | Page-numbering modes: real book pages from page-list, synthetic pages, hidden flows (exclude sections like front matter from progress) | KOR 4.5, 4.7 | M | Needs EPUB `page-list` support |
| R25 | Skim widget: slider to jump through the book and chapters with preview | KOR 4.1 | S | Pairs with R12 |
| R26 | Marking pages for navigation: bookmarks, jump back/forward stack ("go to previous location") | KOR 4.2 | S | |
| R27 | Book map and page browser (thumbnail grid, chapter structure, highlights and bookmarks shown on the map) | KOR 4.3, 4.4 | M | R12 detail |
| R28 | Table-of-contents tools: collapse/expand, alt TOC, TOC from headings when missing, TOC-based progress bar ticks | KOR 4.6 | S | |
| R29 | Highlight modes: normal (drag), select-then-highlight across pages; auto-turn page when selection hits edge; punctuation-aware extension; overlapping highlights picker; adjust highlight start/end after creation | KOR 6.1-6.2 | M | R7 detail |
| R30 | Highlight styles beyond background: lighten, underline, strikeout, invert, colours | KOR, FOL | S | R7 detail; colour palette |
| R31 | Editable highlight text (display text differs from source), reset to original; bookmark list with filters by type/style and "show note" modes | KOR 6.2 | S | Notes view feature |
| R32 | Annotation export targets: txt, html, md, Kindle clippings, JSON; services Joplin, Readwise, Memos, Flomo, XMNote, plus Obsidian/Anki; filename templates; export by highlight style; multi-book export; sidecar import | KOR 6.3, FOL | M | Extends R20 |
| R33 | Status bar: items (clock, battery, page, chapter title, time to read, progress, memory, custom text), order, presets per orientation/book, chapter progress bar, dynamic filler, font choice | KOR 3.3 | M | Extends R15 |
| R34 | Reading profiles: bundle of settings, switchable manually or auto-triggered (by collection, document, time) | KOR 11.3-11.4 | M | Extends R19 |
| R35 | Quick menu: user-built menu of actions, openable by gesture or button | KOR 11.1 | S | Part of R13 |
| R36 | Gesture manager: tap zones, swipes, multiswipes, corner taps, long-press, two-finger gestures, each bound to any action; keyboard shortcut map | KOR 10 | M | R13 detail |
| R37 | Long-press/selection menu: highlight, note, dictionary, Wikipedia, translate, search, copy, share, follow link, open image | KOR, FOL, CAL | S | Mostly present |
| R38 | Dictionary: multiple dictionaries (StarDict/dictd/Wiktionary), dictionary priority and lookup history, custom lookup sources (web URL templates, e.g. Google), word forms/stemming | KOR, CAL 4.x | M | R8 detail |
| R39 | Vocabulary builder: auto-add looked-up words, due-for-review list, flashcard mode | KOR | M | R9 detail |
| R40 | Reading statistics: sessions, per-book/day/week/month, reading speed, chapter/book time-remaining estimates, calendar, configurable "day starts at", export | KOR | M | R11 detail; also feeds estimates in the status bar |
| R41 | Search in book: text, whole word, case, regex, results list with context, next/previous | KOR, CAL | S | Present partly |
| R42 | Reference mode (show original paper-edition page numbers, sync with paper edition) | CAL 4.x | S | Needs `page-list`; shares R24 |
| R43 | Keyboard-only link following (link hints) | CAL | S | Accessibility and desktop power-users |
| R44 | Zoom on images, image viewer with pan, copy image | CAL, KOR | S | |
| R45 | Copy text and images, with attribution/citation template | CAL, FOL | S | |
| R46 | Table-of-contents-driven reading modes: continuous scroll vs paginated, one/two-page spreads, margins, max width | FOL, CAL | M | |
| R47 | Themes: light/dark/sepia/custom with background image/paper texture, per book | FOL | S | |
| R48 | Speed reading: RSVP reader (words per minute, preview words), perception expander lines | KOR 13 | S | Fun differentiator |
| R49 | Auto-turn page on a timer, read timer | KOR | S | |
| R50 | Read-aloud (system TTS) with sentence highlight, voice and speed controls | CAL, FOL | M | Same as R10; viewer-side controls |
| R51 | Fixed-layout EPUB (pre-paginated) rendering | FOL, CAL | L | KOReader does not support it, Foliate does; stretch goal |
| R52 | Comics/manga: CBZ/CBT/CBR reading, RTL page order, panel zoom, double-page spreads, preloading, jump to next file in folder, page refresh settings (e-ink) | KOR 8, FOL | L | Extends R17 CBZ |
| R53 | Book information screen: cover change/reset, description, file info, word/line counts of the page | KOR 14 | S | |
| R54 | Custom cover per book (screenshot a page as cover) | KOR 14 | S | Link with L13 |
| R55 | Footnote popups: configurable (always/never/long-press), back-link behaviour | KOR, FOL | S | R5 detail |
| R56 | Japanese support: vertical writing mode, furigana/ruby, vertical-aware selection, dictionary extensions | KOR (fork only), FOL | L | R4 detail |
| R57 | Open with a different engine, ignore cover/metadata/embedded style per book, reset book data | KOR 9.1 | S | Library context menu |
| R58 | Move-to-archive after finishing | KOR | S | Library action |
| R59 | Page-turn animation toggle, tap-to-turn vs scroll, kinetic scroll, margin tap | FOL, KOR | S | Reader prefs |
| R60 | Frontlight/warmth schedule, auto night mode | KOR 2.5 | S | R14 detail; on desktop = colour temperature/theme schedule |
| R61 | Remote control: HTTP endpoint to drive the reader (next page, open book); mobile companion | KOR | M | Optional; ties to headless mode. Security: auth required, off by default |
| R62 | Text editor built in for sidecar/CSS | KOR | S | Covered by Bookx editor |
| R63 | Reading goals, streak calendar, yearly review | KOR, new | S | R11 detail |
| R64 | Accessibility: screen-reader support, high contrast, dyslexia-friendly fonts (OpenDyslexic, Atkinson), reduced motion | new | M | Needs fonts listed in `NOTICE.md` |

## B2. Library additions (Calibre GUI, KOReader library)

| ID | Feature | From | Effort | Notes |
|---|---|---|---|---|
| L14 | Add books: single files, folders, recursive folders (one book per folder or per file), archives (ZIP/RAR/7z) with multiple books, add by ISBN, add empty record, add formats to existing record, add data files to a record | CAL 1 | M | Extends current import |
| L15 | Guess metadata from file names with a configurable regex | CAL | S | |
| L16 | Edit metadata one-by-one and in bulk (search-and-replace across fields, templates), copy/paste metadata, merge records, manage data files | CAL | M | Bulk edit exists in part |
| L17 | Cover grid, bookshelf view (spines), cover browser (flow) alternative views | CAL, KOR cover browser | M | Library views; the default stays the clean grid |
| L18 | Tag browser: hierarchical categories, drag-and-drop to assign, "not" filter, counts, per-column categories | CAL | M | Partly present |
| L19 | Quickview: see other books sharing a value (author/series/tag) next to the current book | CAL | S | |
| L20 | Notes attached to authors, series, tags, publishers (rich text) | CAL 1 | M | |
| L21 | Temporarily marked books ("mark" and search for marked) | CAL | S | |
| L22 | Search language: full Calibre syntax (field:value, regex, date ranges, relative dates, numeric comparisons, `#custom:`, `vl:`, `search:`, `marked:`), saved searches, search-as-you-type | CAL | M | Query engine exists; fill gaps |
| L23 | Virtual libraries with extra restrictions combinable in searches | CAL | S | L4 detail |
| L24 | Template language for display, sort and save paths (functions, `{title}/{author}`, formatters, conditionals) | CAL | L | Needed for save-to-disk and columns of type "composite" |
| L25 | Custom column types: text, comma-separated tags, long text, rating, date, integer, float, yes/no, composite (template), series-like, enumeration; column rules and colouring | CAL | M | L3 detail |
| L26 | Save to disk with templates, send to device with templates | CAL | S | Needs L24 |
| L27 | Jobs panel: background tasks with progress, cancel, logs | CAL | S | Shared with C10 |
| L28 | Catalogs: generate AZW3/EPUB/MOBI/CSV/XML/BiBTeX catalogs of the library with sections, prefixes, exclusions, custom covers | CAL | L | Tied to D4: only formats Calibre generates |
| L29 | Collections, file-browser shelves, history, favourites, folder shortcuts; multi-select actions | KOR 9 | S | |
| L30 | Reading status per book (new/reading/on hold/finished), finished-date, ratings, reviews | KOR | S | L5 detail |
| L31 | Book metadata archive/storage: sidecar files per book vs central store; portable export of reading state | KOR 9.7-9.8 | M | Under rusqlite: provide export/import of sidecars for KOReader interoperability |
| L32 | Library maintenance: check library, restore database from folder, backup metadata to file, export/import library | CAL | M | L9 detail |
| L33 | Full-text search of all books with index control (which formats, rebuild, status) | CAL | M | L2 detail |
| L34 | Authors: author sort, aliases, link to Wikipedia/Goodreads/OpenLibrary pages | CAL | S | |
| L35 | Identifiers (ISBN, DOI, MOBI-ASIN, Goodreads, Google) and links | CAL | S | |
| L36 | Keyboard shortcuts for every library action, customizable | CAL | S | |
| L37 | Library-level undo for delete/metadata edits | new | M | Differentiator over Calibre |
| L38 | Optimizing documents helper: warn about slow/badly-formed books, suggest fix | KOR 9.4 | S | Uses E8/C8 |

## C2. Editor additions (Calibre editor)

| ID | Feature | From | Effort | Notes |
|---|---|---|---|---|
| E11 | File browser panel: rename (with link refactor), merge, reorder spine, mark cover, delete, export, add new images/fonts/files, replace file, link stylesheets efficiently | CAL | M | Partly done |
| E12 | Search and replace: regex, whole file/selected/open files/all text files, saved searches, function mode (scripts), ignoring HTML tags | CAL | L | Find/replace exists in basic form; function mode uses Rhai after Wave 2 |
| E13 | Checkpoints (named snapshots) and compare/revert | CAL | M | Pair with E4 |
| E14 | Live preview panel synced with code cursor, plus live CSS panel (styles applying to the selected element) | CAL | L | Uses the Bookx reader engine |
| E15 | Split HTML files visually, split at headings, at page breaks | CAL | M | E9 detail |
| E16 | TOC editor: generate from headings/links/files, drag reorder, preview | CAL | M | |
| E17 | Check book: all error classes (missing files, bad links, bad CSS, unmanifested files, OPF errors, duplicate ids, …) with auto-fix | CAL | L | E8 detail |
| E18 | Automated tools: add cover, embed referenced fonts, subset fonts, smarten punctuation, transform CSS properties, remove unused CSS, fix HTML, beautify, insert inline TOC, set semantics, filter style info, upgrade internals (EPUB 2→3 etc.) | CAL | L | E1 detail |
| E19 | Make the book accessible (EPUB accessibility metadata, alt-text checks, landmark/semantics) | CAL | M | E8 detail |
| E20 | Spell check with project dictionaries, add words, per-language, with suggestions | CAL | M | E5 detail |
| E21 | Insert special characters panel; code inspector; external-link checker; download external resources; arrange files into folders by type; import other formats as EPUB; Reports tool | CAL | M | E3 detail |
| E22 | Code editor features: syntax highlighting, context-sensitive help, auto-complete, snippets, tag auto-close | CAL | M | E6 detail |
| E23 | Edit metadata (OPF) in a form: titles, authors, series, identifiers, languages, cover, direction, accessibility fields | CAL | S | |
| E24 | Editing of formats beyond EPUB: AZW3/KF8 (via unpack and repack), KEPUB | CAL | L | Tied to D4. Calibre edits EPUB, AZW3 and HTMLZ |
| E25 | Compare two books/directories with a visual diff, and merge changes | CAL | M | E4 detail |

## D2. Conversion additions (Calibre conversion)

Part 1 ID table C1-C10 stays. The Calibre conversion manual lists these option groups; each becomes a transform stage with tests.

| ID | Feature | Effort | Notes |
|---|---|---|---|
| C11 | Look and feel: base font size, font rescaling (key size), line height, text justification, remove paragraph spacing and indent, extra CSS, embed fonts, change case, smarten punctuation, unsmarten punctuation, input/output character encoding, margins, keep ligatures, linearize tables, transform CSS and HTML rules, filter styles, insert blank line | L | Order matters; port the documented semantics |
| C12 | Heuristic processing: unwrap lines, delete blank lines, italicize patterns, markup chapter headings, replace soft scene breaks, remove fake margins, replace page headers/footers | L | Extends C8 |
| C13 | Page setup and structure detection: chapter detection by XPath, page breaks before, remove first image, insert metadata page, flatten TOC, TOC filter, max levels, force-use auto TOC, number of links | M | |
| C14 | Search and replace regex stage (input-side and structure-aware) | S | |
| C15 | Metadata step: set/override title, author, series, cover, comments, tags, language, publisher on convert; read from OPF; use cover as first page | S | |
| C16 | Debug pipeline output and save intermediate stage | S | Helps independent tests (D4) |
| C17 | Input formats Calibre accepts: AZW, AZW3, AZW4, CBZ, CBR, CBC, CHM, DJVU, DOCX, EPUB, FB2, FBZ, HTML, HTMLZ, KEPUB, LIT, LRF, MOBI, ODT, PDF, PRC, PDB, PML, RB, RTF, SNB, TCR, TXT, TXTZ | XL overall | In scope under D4 and D1 in waves: easy first (TXT, MD, HTML, HTMLZ, FB2, FBZ, DOCX, ODT, RTF, CBZ), then MOBI/AZW3 family, then legacy (LIT, PDB, PML, RB, SNB, TCR, CHM). PDF and DJVU later (D3) |
| C18 | Output formats Calibre produces: AZW3, DOCX, EPUB, FB2, HTMLZ, KEPUB (via plugin in Calibre; we ship natively), LIT (no), LRF (legacy), MOBI, OEB, PDB, PDF, PMLZ, RB, RTF, SNB, TXT, TXTZ, ZIP | XL overall | D4 rule: implement only formats Calibre can really write; prioritise EPUB, AZW3, MOBI, DOCX, FB2, HTMLZ, TXTZ, PDF (later), RTF |
| C19 | Comic input to EPUB/AZW3 with device-optimised image processing (grayscale, dither, split spreads, manga mode) | M | From Calibre comic input options and KOReader manga optimisation guidance |
| C20 | Per-format output options (EPUB version, flat output, no SVG cover, split large files; MOBI prefer author sort, share not sync; AZW3 no inline TOC; PDF paper size, custom size, margins, header/footer templates, serif/sans/mono font families) | M | |
| C21 | Conversion presets (saved option sets) per device and per input format | S | |

## E2. Devices, sharing and servers additions

| ID | Feature | From | Effort | Notes |
|---|---|---|---|---|
| P10 | Content server: browse library, search, read in browser, download, user accounts and permissions per library, HTTPS, URL prefix and virtual-host reverse-proxy setup, run as a systemd service, offline support (PWA), mobile-friendly | CAL server | L | Extends P8; see D7 |
| P11 | Server: OPDS feed with search (including searching inside book content), pagination, cover thumbnails, comics reading in OPDS apps | CAL, KOR | M | P4 detail |
| P12 | Server: KOReader progress sync endpoint compatible with KOSync, and Calibre-style reading progress sync with KOReader | CAL server, KOR | M | P3 detail |
| P13 | Calibre plugin interop in KOReader: wireless send, metadata search via calibre library files | KOR | M | Understand and be a counterpart for KOReader's Calibre plugin |
| P14 | Device drivers beyond mass storage: Kobo (with KEPUB and DB shelf/reading-state sync), Kindle (MOBI/AZW3/KFX handling), PocketBook, Boox, Android MTP, generic folder | CAL | L | Per-device test fixtures; P2/P7 detail |
| P15 | Send-to-device options: main memory/card A/B, format selection and fallbacks, auto-convert on send, templates for save paths, overwrite rules, remove books from device, sync device reading state back to library | CAL | M | Needs L24 |
| P16 | Connect/share: email, folder share, "share" menu, wireless sharing over local network | CAL | M | |
| P17 | KOReader-style file-browser for remote: FTP/SSH-less transfer; WebDAV/Dropbox cloud storage client; cloud sync of books and settings | KOR | M | P9 detail |
| P18 | OPDS client: add catalogs, downloads with format preference, search, auth, pre-set catalogs (Gutenberg, Standard Ebooks, Project Gutenberg, Feedbooks, ManyBooks) | KOR, CAL | M | S3 detail |
| P19 | Wallabag integration and read-it-later sync | KOR | M | S1 detail |
| P20 | QR code share of library URL | KOR | S | Nice for server mode |
| P21 | Remote HTTP control of the reader (see R61), off by default, auth required | KOR | S | |

## F2. Content sources additions

| ID | Feature | From | Effort | Notes |
|---|---|---|---|---|
| S4 | News fetch: recipes (declarative: feeds, cleanup rules, print-version URL rewriting, article style replacement, preprocessing hooks), scheduling, per-recipe accounts, output as EPUB/AZW3 | CAL news | L | S2 detail. Recipes as data (TOML/Rhai), not Python; import-assist for Calibre recipes is optional |
| S5 | RSS/Atom to HTML/EPUB downloader, per-feed settings, "download full article" | KOR news downloader | M | Shares S4 core |
| S6 | Metadata sources: configurable list and priorities per field (Amazon, Google Books, Open Library, ISBNdb, Edelweiss, Overdrive, Douban, Biblioteca, etc.), identify-by-ISBN/title/author, cover download | CAL | L | L6 detail. Respect each site's terms |
| S7 | Metadata from file (OPF sidecar, embedded) and write metadata back to files | CAL | M | Write-back into EPUB/AZW3/PDF |
| S8 | Wikipedia article saved as book/offline lookup | KOR | S | |

## G2. Platform additions

| ID | Feature | From | Effort | Notes |
|---|---|---|---|---|
| X8 | Plugin API (after Wave 2): WASM plugins for input/output formats, metadata sources, device drivers, store actions, UI actions; plugin manager with enable/disable | KOR 12, CAL | XL | X3 detail. Replaces KOReader user patches and Calibre plugins |
| X9 | User patches / hooks: small scripts that tweak behaviour at start-up (Rhai or WASM) | KOR 12.3 | M | After X8 |
| X10 | Profiles + Quick menu + gestures as shareable config files (TOML); import/export of settings | KOR | S | |
| X11 | Preferences coverage mirroring Calibre (look & feel, behaviour, import/export, searching, advanced), but organised in a single searchable page | CAL | M | Calibre's weakness is clutter; keep one page with search |
| X12 | Command-line tools: `bookx convert`, `meta`, `check`, `serve`, `import`, `search`, `list`, `add`, `remove`, `set_metadata`, `export`, `catalog`, `saved_searches`, `custom_columns`, `fetch-news`, `polish`, `embed-fonts`, `subset-fonts`, `debug` | CAL CLI | M | X2 detail; `--output json` everywhere |
| X13 | E-ink mode: no animation, paged menus, high-contrast themes, refresh-friendly rendering, large touch targets | KOR | M | After R13 (D8) |
| X14 | Keyboard-first operation and command palette | new | S | Differentiator |
| X15 | Telemetry-free, offline-first, no accounts; data folder is plain and portable | new | S | Product principle |
| X16 | Release channels, Flatpak, AppImage, `.deb`/`.rpm`, Windows/macOS later; reproducible builds | new | M | X7 detail |
| X17 | Localization workflow (gettext), RTL UI, translations from Weblate | KOR | M | X6 detail |
| X18 | Performance harness extended: open-time, scroll fps, search latency, import throughput, conversion throughput, memory ceiling per book; budgets fail CI | new | M | X4 detail |
| X19 | Diagnostics: "Help → system information", debug log export, safe mode, crash-safe autosave | KOR, CAL | S | |
| X20 | Foliate-style `--doc` open from command line and `bookx://` links, `.desktop` MIME registration for all supported formats | FOL | S | |

## H. Foliate-specific items (consolidated)

Foliate's own list (README and app): EPUB, MOBI, KF8 (AZW3), FB2, CBZ and PDF (via pdf.js), continuous or paginated view, auto-hyphenation, multiple highlight colours, annotations with export, footnote popups, cross-references, dictionary lookup with Wiktionary and Wikipedia, text-to-speech, bookmarks, full-text search, layout controls (font, margins, spacing, brightness, themes with custom colours), quick-notes, selected text actions, library with covers and progress, OPDS browsing, sidebar with TOC, annotations and bookmarks, jump-to-location history, RTL and vertical writing, fixed-layout, media overlays, per-book theme settings.

Map: R2, R3, R4, R5, R7, R8, R10, R13, R17, R18, R20, R30, R46, R47, R51, R55, R56, R59, P4, P18, X20 (all already listed above). New from Foliate only:

| ID | Feature | Effort | Notes |
|---|---|---|---|
| F1 | Sidebar with tabs (TOC / annotations / bookmarks / search results) in the reader | S | Matches existing chrome |
| F2 | Quick note: selection menu with "Add note" in one tap, notes in popover | S | Already done in Bookx |
| F3 | Cross-reference preview: hovering or tapping internal links shows target content | S | R5 detail |
| F4 | Colour-coded highlights with a legend and filter by colour | S | R30 |
| F5 | Jump history (back/forward through positions) | S | R26 |
| F6 | Bookmark shortcut (one key), with an automatic "last position" bookmark | S | |
| F7 | Zoom/pan for images with open in an external viewer | S | R44 |

## I. Suggested priority layering (optional, owner decides)

This does not override D1 (everything in). It only suggests order inside waves.

1. **Foundation, Wave 0:** X1, X18/X4, `Store` trait on rusqlite (L1), conversion pipeline traits (C1), plugin-ready contracts.
2. **Fidelity and flavour, Wave 1 (core):** R1, R2, R3, R5, R7/R29/R30, R21-R23, R40, L2/L33, L22/L24/L25, C1/C5/C11/C12, P1, P3/P12, P4/P11, X12.
3. **Library parity, Wave 1:** L14-L21, L23, L26-L28, L30-L32, S6/S7.
4. **Editor parity, Wave 1:** E11-E25.
5. **Formats, Wave 1:** C2/C3/C17/C18 for formats Calibre supports (D4); CBZ (R52); AZW3/MOBI writers with independent tests.
6. **UX/platform, Wave 1:** R13, R34-R36, R47, R59, R60, R64, X5, X10, X11, X14, X17.
7. **After Wave 2:** X8/X9 plugins, PDF family (C4, PDF reflow, R17-pdf), e-ink mode X13 (after R13), RSVP/perception extras R48, remote control R61/P21.

## J. Verification checklist for every agent

Before starting a track:
1. Read Part 2 of this document first, then this appendix.
2. Open the cited Calibre manual or KOReader user-guide/wiki page for each item and note differences from the row here. If the page was not read in this research (see "Not verified"), read it first.
3. Write the failing tests first (AGENTS.md: logic as pure functions, UI behaviour as `build-aux/ui-tests.sh` scenarios).
4. For D4 formats, write tests independent of the implementation (golden files, third-party validators), and only for formats Calibre supports.
5. Do not copy KOReader code (AGPL). Reimplement from documented behaviour.
6. Add assets to `NOTICE.md`; add docs; run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `build-aux/ui-tests.sh`.


---

# PART 4: Progress log (updated as work lands)

> **2026-11-22:** the "Not done" lists in this log are superseded. The current status per row is `docs/roadmap-status.md` (fully re-audited 2026-11-22) and `docs/calibre-editor-gap-analysis.md`; what is left overall is `docs/remaining-work.md`.

Legend: DONE = implemented and tested; PARTIAL = some of the item; TODO = not started.

## Wave 0: Foundation (2026-11-12) DONE

| Item | Status | Where | Tests |
|---|---|---|---|
| X1 workspace split | DONE (first cut) | `crates/bookx-core` (GTK-free engine), `src/` (GTK app, re-exports the engine as `crate::core`). Further crates are split out per track | whole workspace builds, clippy clean, UI suite unchanged (28 checks) |
| Trait contracts | DONE | `crates/bookx-core/src/contracts.rs`: `FormatReader`, `FormatWriter`, `Transform`, `MetadataSource`, `DeviceDriver`, `Store`, `Registry`, `run_pipeline`, `pick_reader` | 3 unit tests |
| X4 / X18 performance harness | PARTIAL | `crates/bookx-core/tests/perf_budgets.rs` (search, sort, catalogue load, open book, parallel import); `make perf` | 5 budget tests, pass in release. Reader/scroll budgets TODO |
| Ownership map in AGENTS.md | DONE | `AGENTS.md` section 3 | n/a |

Earlier work that already covers roadmap items (before the plan existed): R5-ish (Wikipedia/translate inline), R7 partial (highlight background + notes), R20 partial (JSON/Markdown/text export), L-series basic (JSON catalogue, search language, bulk edit), E-series basic (editor, find/replace, TOC, check), C2 partial (TXT/MD/HTML in, EPUB out, TXT/MD/HTML export), clean library UI (Continue Reading card, cover progress, search bar).

## Wave 1 (2026-11-12): engine crates DONE, GUI integration TODO

Every crate below was written by a separate agent against a testing brief (unit tests for every public function and error path, integration tests, independent validation of file formats, fuzz loops that must not panic, parallel-safe tests) and then re-run by the integrator with `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets -- -D warnings`. None of these crates is wired into the GTK app yet; that is Wave 2.

| Crate | Track | Roadmap IDs | Status | Tests |
|---|---|---|---|---|
| `bookx-store` | T3 | L1, L2/L33 (SQLite FTS5 instead of tantivy), L9 (backup + integrity check, no scheduler), L10, L11, L5/L30 (collections, reading status) | DONE and now the catalogue of the app, the CLI and the server (see the Wave 2 decision); JSON is migrated once | 82 |
| `bookx-formats` | T4 | C2, C3, C17 (LIT excluded, see D9), R17/R52 (CBZ, CBT, CBC; CBR only when the file is really a zip/tar) | DONE: FB2, FBZ, TXT, MD, HTML, HTMLZ, TXTZ, RTF, DOCX, ODT, CBZ, CBT, CBC, MOBI/AZW/AZW3/PRC, PDB (eReader, zTXT stream), PML/PMLZ, RB, SNB, TCR, CHM (LZX). LIT: excluded by the owner (discontinued format): recognised and rejected with a clear message. AZW4: clear error (PDF wrapper, deferred). Not done: LRF, DJVU/PDF (D3), real RAR | 242 |
| `bookx-convert` | T5 (updated later: +DOCX, RTF, PDB, PMLZ, RB, SNB, OEB, ZIP writers, 379 tests; LRF and PDF not done) | C1, C5, C6, C8, C9, C11-C13, C15, C20, C21, E18 (smarten punctuation, remove unused CSS) | DONE: EPUB 2/3, KEPUB, FB2, HTMLZ, TXTZ, TXT, MD, HTML, MOBI and AZW3/KF8 writers, both checked against Calibre as an oracle (structure compared with `ebook-convert` output, `ebook-meta` reads our metadata/cover, `ebook-convert` round-trips text/TOC/images); 11 transforms; pipeline with progress and cancellation; presets. NOT tried on a real Kindle. Known differences from Calibre: no trailing TBS entries, no inline TOC page, one fragment per file (use `split_large_html`), fonts/SVG dropped | 218 (incl. 4 perf budgets) |
| `bookx-text` | T1 | R1 (CSS subset), R2, R3, R7/R29/R30 (data model), R19, R21-R24, R34 (data model) | DONE as pure logic. Not wired into `canvas.rs` yet | 186 + 4 perf budgets |
| `bookx-lookup` | T2 | R8, R38, R9, R39, R11, R40, R63, R41, R48, R26, R31, R32 (partial), R12/R27 (data) | DONE as pure logic, no UI | 175 + 2 perf budgets |
| `bookx-serve` | T6 | P1 (mass storage), P3/P12 (KOSync), P4/P11/P10 (OPDS + content server), P6 (message building), P18 (OPDS client parsing) | DONE. Gaps: no HTTPS, no header-size limit or read timeout (tiny_http), no Kobo DB write, no MTP, no SMTP, no OPDS client networking | 187 |
| `bookx-sources` | T8 | L6/S6 (Open Library, Google Books, Wikidata parsers, ranking, field-priority merge, backoff), S1 (article to EPUB), S4/S5 (RSS/Atom/JSON Feed, recipes, schedules, news EPUB), L13 (cover generation/crop), L7 (duplicates), L15 (filename guessing), L24 (template language subset), L34/L35 | DONE as a library with mock-only tests (no live network tests); nothing wired into the app yet | 283 (261 unit + 18 integration + perf) |
| `bookx-editing` | T7 | E1 (font subsetting/embedding), E2, E3, E4, E5/E20, E7, E9/E15, E13, E17-E19, E21, E22 partial, E25 | DONE as a library (194 tests incl. 12 000 random-garbage inputs and split/merge/arrange property tests; found and fixed a multi-byte slicing bug). Not wired into the editor UI yet. Gaps: no CFF font test, subsetted fonts lose kerning/ligatures, no oxipng, E8 (spec-based validation) not covered | 194 |
| `bookx-cli` | X2/X12 | X2, X12 (partial) | DONE: `formats`, `presets`, `convert`, `meta show/set`, `check`, `library add/list/info/remove/export`, `search`, `serve` | 40 (includes the real binary) |

Bugs found by the new tests and fixed in `bookx-core`:
- `util::title_sort` panicked on titles whose first characters are multi-byte (for example "Война и мир"); this crashed metadata editing in the app. Regression test added.

Performance budgets now exist in `bookx-core`, `bookx-store`, `bookx-convert`, `bookx-text` and `bookx-lookup` (`make perf`).

Integrator gate at the end of Wave 1 (2026-11-12): `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace` 1054 passed / 0 failed, `make perf` green in release, UI suite unchanged.

### Known gaps carried into Wave 2 or later (status as of the end of Wave 1; see the Wave 2 section for what has since been done)
- GUI wiring of everything above (T9 plus the UI parts of T1, T2). DONE in Wave 2: hyphenation, highlight styles UI, dictionary popup, stats page, vocabulary, send to device, share toggle, format import, conversion, full-text search. STILL OPEN: Knuth-Plass in the canvas, CSS override layer, SQLite as the catalogue.
- Tracks not started at the end of Wave 1: T7 editor polish (E-series beyond what exists; STILL OPEN), T8 metadata sources and news recipes (library DONE later as `bookx-sources`, UI still open), T10 plugins (after Wave 2, D5), PDF family (D3), e-ink mode (after R13, D8), TTS (R10), fixed-layout EPUB (R51), vertical writing in the canvas (R4), P5/P7/P9/P14-P17 (Calibre wireless protocol, MTP, WebDAV, Kobo DB, email sending).
- Remaining Calibre input formats: DONE later except LIT binary markup, LRF, PDF/DJVU. Output formats still owed under D4: DOCX, RTF, PDB, RB, SNB, PMLZ, OEB, ZIP (PDF later).
- Security: DONE after Wave 2 started: `tiny_http` was replaced by an own HTTP/1.1 server in `bookx-serve/src/net.rs` with request-line/header/body limits, real read deadlines, connection caps (64 total, 32 per IP), strict parsing against request smuggling, an auth-failure throttle (10/min then 429), `no-store`/`nosniff` headers; 266 tests incl. slowloris and soak. Still open: no TLS, response bodies are buffered in memory.
- Licensing: hyphenation pattern licences are listed in `NOTICE.md`.

## Wave 2 (2026-11-12, in progress): integration into the GTK app

Each item ships with unit tests (pure logic in `src/*.rs`) and parallel UI scenarios in `build-aux/ui-tests.sh` (77 checks at this point, stable over repeated runs with 25 scenarios at once).

| Item | Roadmap IDs | Status | Where |
|---|---|---|---|
| Reading statistics: sessions recorded by the reader (idle, jump and page-turn rules), summary window with today/week/month, streaks, speed, 7-day chart, top books | R11, R40, R63 | DONE | `src/reading_stats.rs`, `src/ui/stats.rs` |
| Hyphenation in the canvas via soft hyphens, canonical offsets so highlights/search/selection are unchanged, Preferences switch, language from book metadata | R2 | DONE | `src/ui/canvas.rs`, `bookx-text` |
| Highlight styles: background, underline, strikethrough, squiggly (invert supported in the canvas, not offered in the UI), per-highlight, remembered for the next one; `Highlight.style` in the catalogue (old catalogues load) | R7, R29, R30 | DONE | `src/ui/canvas.rs`, `src/ui/reader.rs`, `bookx-core` |
| Offline dictionary lookup in the selection popover (StarDict folders, word forms, helpful message when none are installed) | R8, R38 | DONE | `src/dictionaries.rs` |
| Vocabulary builder: lookups saved with definition and context, flashcard review window, SM-2 scheduling | R9, R39 | DONE | `src/vocabulary.rs`, `src/ui/vocab.rs` |
| Library imports every format `bookx-formats` reads (file filters, folders, drag and drop) | C2, C3, C17 | DONE | `src/import.rs`, `bookx-core::library::prepare_from_archive` |
| Convert To KEPUB, AZW3, MOBI, FB2, HTMLZ, TXTZ, EPUB from the library context menu (worker thread, save dialog or folder) | C5, C6, C9 | DONE | `src/export.rs`, `src/ui/library.rs` |
| Share Library: OPDS + web page + optional KOSync, password required for LAN, controller with start/restart/stop, catalogue cache | P3, P4, P10, P12 | DONE | `src/sharing.rs`, `src/ui/share.rs` |
| Send to device: detects Kobo/Kindle/PocketBook/generic readers, converts to the best format (KEPUB, AZW3, EPUB), never overwrites, stays inside the device folder; context-menu entries per detected device | P1 | DONE (USB mass storage; no MTP, no Kobo DB write) | `src/devices.rs`, `src/ui/library.rs` |
| Full-text search of book contents: background indexer into SQLite FTS5 (`bookx-store`), `text:` search term that combines with every other field | L2, L33 | DONE (the catalogue is SQLite too, see the decision below) | `src/fulltext.rs`, `src/ui/library.rs` |
| Fetch metadata in the edit dialog: query from title/authors/ISBN, Open Library + Google Books + Wikidata merged with confidence, fill-empty or replace, cover, status line; canned answers via `$BOOKX_SOURCES_MOCK` for tests | L6, S6 | DONE | `src/metadata_fetch.rs`, `src/ui/metadata.rs` |
| The book's own CSS applied in the reader (italic/bold/underline/strike/small caps/monospace classes, text-align, display:none; differences against the built-in defaults only, so exports stay clean), Preferences switch | R1 (partial), R21/R22 | DONE for these properties; text-indent, margins, colours, fonts, floats, drop caps still open | `bookx-core::html::Styling`, `OpenBook::chapter_doc_styled`, `bookx-text::css` |
| Touch and tablet support: swipe to turn pages (classified by distance/direction/speed), long-press selects a word and a drag extends it (a plain finger drag no longer selects), tap-zone presets, large touch controls (automatic on touch screens), Preferences | R13 | DONE (logic and wiring tested headless; not tried on a real touch screen) | `src/gestures.rs`, `src/ui/canvas.rs`, `src/ui/preferences.rs` |
| E-ink mode (after R13, D8): pure black on white, GTK animations off, bars never fade, tuned prefs (wide tap zones, hyphenation, no scrolling layout) | R14 (e-ink part), X13 | DONE (no frontlight/warmth schedule, no partial-refresh control) | `src/eink.rs`, `src/app.rs`, `src/style.css` |
| Editor Tools menu on `bookx-editing`: fix problems, upgrade to EPUB 3, inline contents page, accessibility, compress images, shrink fonts, arrange files, Book Report, checkpoints (automatic before every tool, restore, a failed tool rolls back) | E1, E2, E3, E13, E17-E19 | DONE for these tools. Not in the UI yet: spell check, compare/diff, split/merge, CSS transforms, link download | `src/editor_tools.rs`, `src/ui/editor.rs` |
| Duplicate finder with keep/merge (metadata union, highlights, bookmarks, furthest progress) | L7 | DONE | `src/duplicates.rs`, `src/ui/duplicates.rs` |
| Add from the web: article to book (readability, images) and feed to news book | S1, S4, S5 | DONE (no recipes editor, no Wallabag) | `src/web_import.rs` |
| News subscriptions: schedule (manual/hourly/daily/weekly, local time), keep-N pruning, per-feed switch, OPML import, Download Now/All Due, 60 s scheduler | S4, S5 | DONE (schedule offset is fixed when saved, so a daylight-saving change shifts the local hour; scheduler runs only while the app is open) | `src/news.rs`, `src/ui/news.rs`, `news_*` UI scenarios |
| Plugins: manager UI in Preferences, plugin metadata sources in Fetch Metadata, plugin readers in the importer | X3, X8 | DONE for metadata sources and readers | `src/plugins.rs`, `bookx-plugin` |
| Plugin writers (Convert To, Send to), transforms (library tools, editor Tools, per-conversion option) and device drivers (`PluginDevice`: host-side listing and writing inside the mount), permissions shown in Preferences; developer guide `docs/plugin-development.md` with worked wasm writer `examples/plugins/rawcopy-writer` (loaded by `tests/example_plugin.rs`) | X3, X8 | DONE. Plugins stay off by default (D5) | `src/plugins.rs`, `bookx-plugin/src/adapters.rs`, `src/ui/preferences.rs` |
| User patches (TOML regex/CSS rules at import-after / convert-before / convert-after / open-before), Preferences list with enable, order and dry run | X9 | DONE (TOML rules instead of Rhai: same sandbox as wasm, nothing to execute). Scenarios `patches_*`, unit tests in `patch.rs` | `bookx-plugin/src/patch.rs`, `src/plugins.rs` |
| Book CSS: first-line indents (em, inherited), paragraph margins (tight indented paragraphs as in print), left margins that add up in nested blocks, all clamped | R1 (more) | DONE; colours, fonts, floats, drop caps still open | `bookx-core::doc::BlockCss`, `src/ui/canvas.rs` |
| EPUB validator `bookx_editing::validate` (87 rules, epubveri-style ids; oracle test runs only when epubveri is installed), font kerning/ligature retention, CFF subsetting test, oxipng; compare with checkpoint in editor; mockups redrawn (library, reader, dialogs, e-ink, editor) | E8, E4 | DONE; validator not yet in the editor UI | `crates/bookx-editing`, `examples/mockups` |
| `bookx-serve` on hyper (own HTTP code removed), optional HTTPS (`--tls-cert`/`--tls-key` in the CLI), SMTP sending with lettre (`mail::send`, not yet in the UI); EPUB validator in Check; Check Spelling (Hunspell); split at headings / merge with next; reader colours Theme / Original / Custom (also in the editor preview) | X2, P17 (engine), E8, E9, E15, R1 | DONE; e-mail is now in the UI (Preferences → E-mail, book menu → Send by E-mail; password kept in the settings file like the share password), no TLS in the app's Share Library | `crates/bookx-serve`, `src/editor_tools.rs`, `src/ui/colors.rs` |

Decision (2026-11-12, owner): JSON is gone, SQLite is the catalogue. (An earlier note here proposed keeping JSON; the owner overruled it: either do it fully or not at all.) Done:

| Item | Detail |
|---|---|
| `bookx-core` | `Library` keeps the books in memory behind a `CatalogBackend` trait (`load`, `commit`); `save()` diffs per-book hashes and commits only changed and removed books plus the id counter. `MemoryBackend` for tests. No JSON code remains in core. |
| `bookx-store` | `open_library(root)` opens `<root>/bookx-library.db` (schema v4: adds `text_stamps`), `SqliteCatalog` implements the trait, `commit_catalog` is one transaction, `CatalogWatcher` lets the sharing server follow commits via `PRAGMA data_version`, `export_catalog_json` backs `bookx-cli library dump`. An old `bookx-library.json` is imported once when the database is new and renamed to `bookx-library.json.migrated`; a damaged one is an error that names the file and is never deleted. |
| App | opens the library through the store; the full-text index now lives in the same database (no separate `fulltext.db` or marks file); switching the library folder reopens both; a toast tells the user about a migration. |
| CLI | `library dump` (full JSON catalogue), migration note, `serve` follows the database. |
| Tests | 14 catalogue tests in `bookx-store` (persistence of every field, edits/removals/id counter, migration incl. damaged and stale JSON, watcher, atomic commit, text stamps), 7 backend tests in core, 5 CLI tests, the UI scenario `migrate_json`, perf budgets: first save of 10 000 books, one edit among 10 000, reopen. The UI tests read the catalogue through `bookx-cli library dump`. |

Wave 2 gate (2026-11-12): `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace` 1538 passed / 0 failed, `make perf` green in release, UI suite 88 checks (27 scenarios in parallel, repeated runs stable), app reinstalled.

Not yet integrated (still library-only): SQLite store as the catalogue (T3, `bookx-store`; only its full-text index is used so far), Knuth-Plass in the canvas (Pango's own breaking is used), user CSS override layer and style tweaks UI (R21), remaining CSS properties (indents, margins, colours, fonts), typography profiles UI, CLI binary packaging, read-later UI, template-based save paths, duplicate finder UI.
Not yet re-drawn: the design mockups (`cargo run --example mockups`) do not show the new dialogs.

## Formats and send-with-notes follow-up (2026-11-12)

| Item | Status | Where | Tests |
|---|---|---|---|
| D4 output formats DOCX, RTF, PDB (eReader), PMLZ, RB, SNB, OEB, ZIP: writers existed in `bookx-convert`; now also in the app's *Convert To* list (`src/export.rs`) with a round-trip test through the readers | DONE | `src/export.rs` | `converts_to_the_remaining_calibre_formats`, `the_new_formats_round_trip_their_text` |
| LRF input (best effort, pure Rust): header, info XML, object index, page tree, blocks, text tags (paragraphs, italic, sup/sub, inline images), image streams, TOC, descrambling, thumbnail as cover. No layout, fonts, buttons or sound; LRX (encrypted) rejected. Verified on real files from Calibre's LRF writer when `ebook-convert` is installed, otherwise on synthetic files | DONE (best effort) | `crates/bookx-formats/src/lrf.rs` | 10 unit tests |
| PDF: opened page by page (`crates/bookx-pdf`, `src/ui/reader_pdf.rs`), reflowed to EPUB on import (`crates/bookx-formats/src/pdf.rs`), kept as a format of the book; OCR not done | DONE (wave 4) | `docs/pdf-research.md` | 62 + 5 perf in `bookx-pdf`, 10 in `bookx-formats`, 19 `crate::pdf`, 1 export, 2 CLI, 1 core; 13 Broadway scenarios, 1 real-session |
| KEPUB spans, MOBI header/EXTH/INDX and AZW3 structural tests | already present (`tests/kepub.rs`, `tests/mobi.rs`, `tests/azw3.rs`) | `bookx-convert` | unchanged |
| Bug 2, send with annotations: *Send to … with Notes* appends an Annotations chapter (spine and contents) before conversion, so it works for KEPUB/AZW3/MOBI/EPUB, and writes a KOReader sidecar (`metadata.epub.lua` bookmarks with text, note, chapter, date) for non-Kindle readers. KOReader xpointers cannot be derived from character offsets, so entries jump to the chapter start | DONE | `src/send_notes.rs`, `src/devices.rs`, `src/ui/library.rs` | unit tests in `send_notes.rs`/`devices.rs`, UI scenario `send_with_notes` |

### Devices and sharing follow-up (2026-11-12)

| Item | Roadmap | Status | Where |
|---|---|---|---|
| Right-click menu always has a device entry (superseded: enhancement 4 moved devices out of the book menu into a header-bar Device button, see `docs/device-ui-notes.md`) | P1 | SUPERSEDED | `src/ui/library.rs`, scenarios `device_none`, `device_button` |
| Device window (menu → *Device…*, book menu): books on the device, remove selected, free space, main memory vs SD card (told apart by the volume name), format picker with fallbacks, save-path templates, Kobo shelf, *Sync Reading State* | P1, P5, P14 | DONE, simulated devices only | `src/ui/devices.rs`, `src/devices.rs`, `bookx-serve::devfs`, scenarios `device_window`, `device_options` |
| Reading state from devices: Kobo `KoboReader.sqlite` (progress, status), KOReader sidecars; matched to library books by title, only ever moves progress forward | P14 | DONE for Kobo and KOReader. Kindle's own binary `.sdr` data is not read (KOReader on a Kindle works) | `bookx-serve::{kobo,reading}`, `devices::plan_sync` |
| Kobo database: read shelves, write a shelf for sent books (backup first, one transaction, refuses other databases) | P15 | DONE against built databases; **never run on a real Kobo** | `bookx-serve::kobo` (8 tests) |
| Share Library over HTTPS: own certificate/key files or a self-signed pair made with `rcgen`, kept in the data folder, regenerate button | P4 | DONE | `bookx-serve::cert`, `src/sharing.rs`, `src/ui/share.rs`, scenarios `sharing_https`, `sharing_https_files` |
| Real-client HTTPS test of `bookx-cli serve --tls-cert/--tls-key` (separate process, rustls client: trusted, untrusted, wrong host name, plain HTTP) | X2 | DONE | `crates/bookx-cli/tests/https_client.rs` |
| WebDAV / cloud folder as a device (PROPFIND, MKCOL, PUT without overwriting, DELETE, quota). Own 150-line blocking HTTP client because `ureq` refuses PROPFIND/MKCOL and the WebDAV crates pull in `reqwest` | P9 | DONE against an in-process server; not run against Nextcloud | `bookx-serve::{webdav,httpc}` |
| MTP: `mtp-rs` (pure Rust, `nusb`) exists but needs exclusive USB access that gvfs already holds, and udev rules; so the driver reads gvfs mounts (`/run/user/<uid>/gvfs/mtp:*`). Limits: no fsync, free space often unknown, locked phones show nothing, no Kobo database | P7 | DONE for gvfs; direct USB MTP not done | `bookx-serve::mtp` |
| Calibre wireless "smart device" protocol | P14 | NOT DONE: plan only | `docs/calibre-wireless-device-plan.md` |

Not done: Kindle's native reading state, direct-USB MTP, Calibre wireless, any test on real hardware.


## Library health, safe file moves, trash and undo (2026-11-12)

Pain points: PP-C-1, PP-C-2, PP-C-3, PP-C-7, PP-C-16, PP-C-24, PP-G-1/2, PP-G-3, PP-G-4, PP-G-12, PP-G-13, PP-G-32, PP-G-33 (numbers as in `docs/pain-points-*.md`).

| Item | Status | Where | Tests |
|---|---|---|---|
| Backup, restore, integrity check wired: *Library Health* window, `library backup/snapshots/restore/check` | DONE | `store/health.rs`, `src/ui/health.rs`, `bookx-cli/libtools.rs` | 10 store tests, 12 CLI tests, scenarios `health_backup_restore`, `health_check` |
| Automatic rolling snapshots (start-up and exit, keep N + one per day for a week, configurable and switchable, skipped when nothing changed) | DONE | `health::{auto_snapshot,retention}`, `ui::health::{on_startup,on_exit}` | retention unit tests, scenarios `health_auto_snapshots`, `health_auto_off` |
| Calibre import with progress and report; links in place or copies into the library | DONE (GUI and `library import-calibre`) | `store/calibre.rs`, `Library::copy_into_library` | store + CLI tests, scenarios `health_calibre`, `health_calibre_link` |
| Transactional moves: plan, dry run, rollback on any failure, errors surfaced; layout flat / Author-Title tree / template / linked, in Preferences and `library layout` | DONE | `core/{fsops,layout,files}.rs` | 6 fsops + 8 layout unit tests, 14 library tests incl. injected failures, scenarios `health_layout*` |
| Network / synced folder warning (once per library), single-writer lock with a clear message, read-only mode | DONE for Linux (`/proc`); other systems: lock only, assumed alive | `store/guard.rs` | 10 unit tests, scenarios `health_readonly`, `health_stale_lock`, `health_sync_warning` |
| Operation log with undo and a trash (restore, empty) | DONE for import, delete, metadata edit, layout move. Not undoable: editor saves (the editor has checkpoints), merge of duplicates (the deleted copies are in the trash, the merged metadata stays) | `core/{history,files}.rs` | library tests, scenarios `health_trash`, `health_undo_empty` |
| Watched import folder with a review queue | DONE while the window or start-up looks at it; no background watcher yet (it is rescanned at start-up and when the window opens or *Check Now* is pressed) | `core/inbox.rs` | 5 unit tests, CLI test, scenario `health_inbox` |
| Name tidying (`Last, First`, case, one spelling per author and series), series index parser (`1`, `01`, `I`, `Book Two`, `#3`, `3rd`), number words and Roman numerals in the duplicate finder | DONE; the series index parser is not yet called by the metadata editor | `bookx-sources/{tidy,dedup}.rs` | 8 + 1 unit tests, perf budget, scenario `health_tidy` |
| Series view hook | DONE as data (`tidy::series_overview` with missing numbers, shown in the Health window); no series page in the library | same | unit test |
| Provenance per field + drift check file vs catalogue | DONE for the edit dialog: fields filled by *Fetch Metadata* are recorded with their source (`openlibrary`, …), other changed fields as `you`, shown as *Sources* in the details panel and in `library info`; no record means the value came from the file. Not recorded: bulk edits, tidy, the editor's metadata tab, plugin patches. Drift check: `check --drift` with *use file* / *use catalogue* fixes | `store/provenance.rs`, `health::drift_fields`, `library_health::record_edit` | 2 + 1 + 5 tests, scenario `health_provenance` |
| 100 000 book budget: first save, one-edit save, unchanged save, search, sorted ids, snapshot, integrity check, reopen, library search | DONE | `bookx-store/tests/perf_budgets.rs` | `a_hundred_thousand_book_library` (about 15 s in a debug build) |

Not done: provenance for bulk edits and the editor; a background folder watcher (`notify`); Windows/macOS network detection; library lock over NFS (it is advisory and a pid check, which is why the network warning exists); undo of editor saves.


## Reader typography, style layer, fixed layout and read aloud (2026-11-12)

Roadmap IDs: R3, R1 (floats, drop caps), R21, R10, R51, R4 (RTL only), bug 1/6 of `docs/bugs.md`.

| Item | Status | Where | Tests |
|---|---|---|---|
| Knuth-Plass line breaking in the canvas. The text is measured on one line with Pango, `bookx_text::paragraph::plan` breaks it (Knuth-Plass, hyphenation points from the soft hyphens), the breaks are forced into the text as ghost characters (U+2028, U+2010 for the hyphen; they never count as book text, so highlight offsets, search and copy are unchanged) and every line is justified with letter spacing on its spaces. Falls back to Pango when the plan does not come out as planned (line count differs), for right-to-left text, markers/code/ragged text, and when switched off (`ReaderPrefs.optimal_breaks`, Style Tweaks) | R3 DONE | `bookx-text/src/paragraph.rs`, `src/ui/canvas.rs` (`optimal_segments`) | 14 unit tests in `paragraph`, 5 in `canvas::break_tests`; scenarios `optimal_breaks` (50 of 50 lines reach the full width, canonical length unchanged, fallback), `hyphenation`; budgets: `planning_optimal_breaks_for_a_chapter_is_fast`, `paginating_a_long_chapter_and_turning_pages_is_fast` in `bookx-text/tests/perf_budgets.rs`, scenario `reader_budget` (layout time and 60 page turns of the real canvas) |
| Drop caps and floats via the CSS subset: `::first-letter { float: left }` gives a drop cap 2 to 6 lines tall (the letter stays in the text as an invisible dot, so offsets do not move; the big initial is drawn beside the lines), floated images (`float: left/right` on the image or a figure around it) with the text wrapping around them over as many lines as the picture is tall, also across paragraphs. A wrapped paragraph is laid out in two pieces (narrow head, full-width tail) that count as one paragraph | R1 DONE for these; images are limited to 45% of the column and 70% of the height | `BlockCss.drop_cap`, `Span::Image.float`, `FloatSide` in `bookx-core/doc.rs`, `html.rs`; canvas `Zone`/`FloatBox`/`segments` | 6 in `html::float_tests` (incl. the user layer), 5 in `break_tests`, 1 in `paragraph`; scenario `drop_cap_and_float` (also screenshot checked) |
| User CSS override layer and Style Tweaks dialog: line height, margins, paragraph spacing, font, force justification, optimal breaking, hyphenation language (book language / off / a language), user style sheet text box; for all books or per book (a book with its own tweaks ignores the general ones); saved in `config.json` (`tweaks.general`, `tweaks.books`); the style sheet goes through `Styling::layered` (the user's rules win over the book's) and `OpenBook::chapter_doc_layered` | R21 DONE | `src/tweaks.rs`, `src/ui/reader_style.rs`, main menu → *Style Tweaks…* | 8 in `tweaks`, the user-layer test in `html::float_tests`, a config test, scenario `style_tweaks` (drives the real dialog) |
| Right-to-left text: Pango lays Arabic/Hebrew out right to left and aligns it; Knuth-Plass is skipped for it (it measures left to right) | R4 PARTIAL: RTL checked | canvas | scenario `rtl_text` (Arabic book, screenshot checked) |
| Vertical writing: `writing-mode: vertical-rl | vertical-lr` (also `-epub-`/`-webkit-` and the old `tb-rl`), `text-orientation: mixed | upright | sideways`, `text-combine-upright` (tate-chu-yoko), ruby (furigana, vertical and horizontal), kinsoku, punctuation squeezing, page turns for right-to-left books, scrolling sideways, selection / highlights / search / read-aloud highlight in the transposed geometry. The chapter is laid out in a logical frame exactly as before (lines along `u`, flow along `v`); `bookx_text::vertical::Frame` maps it to the screen in four places only (drawing, hit testing, rectangles, scrolling). Pango does the glyphs: a context with base gravity East and gravity hint Natural makes ideographs upright (with the `vert` forms of the font) and Latin turned, and the page is drawn line by line in a cairo frame turned a quarter. Details and what is left: `docs/vertical-writing-plan.md` | R4 DONE (vertical, ruby, tcy, kinsoku); R56 DONE except the dictionary extensions | `bookx-text/src/vertical.rs`, `bookx-text/src/css/` (writing-mode, text-orientation, text-combine-upright), `bookx-core` (`doc.rs`, `html.rs`, `book.rs`), `src/ui/canvas.rs` | 23 unit tests in `bookx_text::vertical` and 2 in the CSS values, 10 in `bookx-core::html::vertical_tests`, 5 Pango tests in `canvas::vertical_tests`, 1 in `a11y` (sentences with Japanese brackets), budget `vertical_pagination_kinsoku_and_tate_chu_yoko_scale_to_a_long_book`; Broadway scenarios `vertical_layout`, `vertical_chapters`, `vertical_scroll_and_styles`, `vertical_read_aloud`, `vertical_budget`; real-session scenarios `vertical_reading` (real drag, highlight, search, AT-SPI character boxes) and `vertical_aozora` (a real novel). Not done: hanging punctuation, `line-break`/`word-break`, emphasis dots, vertical tables, floats in vertical text, Knuth-Plass for vertical lines (Pango's own justification is used) |
| Fixed-layout EPUB: `rendition:layout` = `pre-paginated` (or the Kindle `fixed-layout` meta) shows one spine item per page, single column, small margin, pictures scaled up or down to fit the screen and centred | R51 PARTIAL | `bookx_core::doc::is_fixed_layout`, `OpenBook::is_fixed_layout`, `CanvasStyle.fixed` | 2 unit tests, scenario `fixed_layout`. Not done: spreads (two pages side by side), text pages with absolutely positioned elements (they flow as normal text), SVG pages with embedded text |
| Read aloud: sentence by sentence through speech-dispatcher, sentence highlighted (and scrolled to), play/pause/stop, speed (0.5x to 2.5x), voice (voices of the book language), goes on with the next chapter, starts at the selection or the top of the page. The route is an own SSIP client (a line protocol on speech-dispatcher's Unix socket, daemon started on demand) instead of the `tts` crate or `speech-dispatcher-sys`: those link `libspeechd` (headers at build time, a matching library at run time); the socket client has no native dependency, works from a Flatpak that has the socket, and is tested against a fake daemon. Not tried: other back ends (macOS, Windows) | R10 DONE on Linux; engines replaced and extended by enhancement 9 (2026-11-13): the speech-dispatcher code moved to `crates/bookx-tts/src/speechd.rs` (removed entirely by enhancement 20, wave 4) and the voices are Kokoro/Piper over ONNX Runtime or an external server, see `docs/tts-research.md` and `docs/tts-notes.md` | `crates/bookx-tts`, `src/tts.rs`, `src/ui/reader_tts.rs`, `src/ui/tts_settings.rs`, `reader.rs` | 18 unit tests incl. a fake SSIP daemon (kept), scenarios `tts_read_aloud`, `tts_chapters` (silent mock via `BOOKX_TTS_MOCK`). Also tried by hand against the real daemon's protocol (client name, notifications, voice list) |
| Bug 1 (look-ups in the reader not working) and bug 6 (double click) | FIXED | `src/ui/reader.rs`, `canvas.rs` | scenarios `dictionary_button`, `wikipedia_button`, `translate`; unit tests `release_tests`. Cause: the popover changed size after it was shown and vanished; the release of a double click counted as a click and cleared the selection |

Known limits of this wave: the drop cap letter is not painted by highlights or the selection (the dot is); a float that crosses a page break is cut by it; the drop cap is only for the first letter when it is a letter or digit; read aloud speaks the text as laid out in the chapter (no pronunciation control, no SSML).


## Roadmap audit and gap pass (2026-11-13)

The full audit of every row against code, tests and wiki is `docs/roadmap-status.md` (176 rows: 65 DONE, 4 ENGINE ONLY, 63 PARTIAL, 41 MISSING, 3 DEFERRED before; 75 / 3 / 64 / 31 / 3 after this pass, E-series rows excluded). Use that file, not this log, for the current status of a row.

| Item | Roadmap IDs | Status | Where | Tests |
|---|---|---|---|---|
| Saved searches and virtual libraries: `search:name` / `vl:name` terms (nesting, `-` negation by De Morgan, cycle and size checks), a virtual library narrows every view, browse-panel group, menu window | L4, L22 (part), L23 | DONE | `src/saved_searches.rs`, `src/ui/saved.rs`, `src/ui/library.rs` | 9 unit tests, scenario `saved_searches` (11 checks) |
| Online catalogues (OPDS client): presets Gutenberg and Standard Ebooks, own catalogues with basic auth (password in the keyring, sent only to that host), folders, Load more, search by template or OpenSearch description, best-format download, normal import | P4c, P18, S3 | DONE (not run against the live services) | `src/opds_browse.rs`, `src/ui/opds.rs` | 8 unit tests, scenarios `opds_browse`, `opds_errors` with canned pages (`BOOKX_OPDS_MOCK`) |
| Footnote popups (note detection by `epub:type`/`role`/list item/id, number and back-arrow stripped, long targets followed), on/off in Preferences | R5, R55, F3 | DONE | `src/footnotes.rs`, `src/ui/reader_extra.rs` | 6 unit tests, scenario `footnote_popup` |
| Speed reading window (ORP focus letter, preview, speed 50-1000 wpm, keyboard, remembered speed) | R48 | DONE | `src/rsvp_view.rs`, `src/ui/rsvp.rs` | 7 unit tests, scenario `speed_reading` |
| Reading goals, year calendar, yearly review in the statistics window | R63, R11, R40 (part) | DONE | `src/goals.rs`, `src/ui/stats.rs` | 4 unit tests, scenario `reading_goals` |
| Translation infrastructure: `tr()`/`N_()` marks, embedded `.po` (es, de) plus `.mo` override from locale dirs, system language detection, Preferences → General → Language, `build-aux/i18n.sh` writes `po/bookx.pot` | X6, X17 | PARTIAL: only a few windows are marked | `src/i18n.rs`, `po/`, `src/ui/language_settings.rs` | 9 unit tests (parsers, `.mo` round trip, template up to date, languages complete), scenario `language` |

Not done and why: custom columns (needs the store schema, in the metadata agents' area), keyboard shortcut rebinding, image viewer (canvas is shared with the vertical-writing work), Calibre wireless (plan in `docs/calibre-wireless-device-plan.md` is multi-day), collections UI. The remaining top 15 are listed at the end of `docs/roadmap-status.md`.
