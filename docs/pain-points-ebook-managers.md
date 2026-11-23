# Pain points of ebook library managers and readers (general, cross-product)

Research for `docs/enhancements.md` item 2. Written 2026-11-12. Research only: no code was changed.

Status: finished, pending move to archive/ (still pointed to from enhancements.md or source comments) | Last reviewed: 2026-11-23 | Owner: area agent, reviewed by @adhadse

## Contents

- [Summary table](#summary-table)
- [Audit 2026-11-20 (current status; the paragraphs below are the research-time state)](#audit-2026-11-20-current-status-the-paragraphs-below-are-the-research-time-state)
- [The points](#the-points)
- [Top 10 things Bookx should do next](#top-10-things-bookx-should-do-next)
- [Differentiators Bookx could own](#differentiators-bookx-could-own)
- [Unverified claims and gaps in this research](#unverified-claims-and-gaps-in-this-research)

Method: web search of issue trackers, forums, release notes and reviews (links per point), then a check of
the Bookx code and `docs/bookx-feature-roadmap.md` (Part 2 owner decisions first, then Part 4 progress log).
Calibre-only complaints belong to item 1 of `enhancements.md`; Calibre appears here only where the pain is
shared with other products.

How to read it:

- **Frequency**: high = seen across 3 or more products or in many threads; medium = 2 products or a few threads;
  low = a niche or a single product. I judged this from the sources I opened, not from counts.
- **Bookx status**: DONE / PARTIAL / NOT YET, verified against the code or a roadmap row (cited). "Roadmap" means
  `docs/bookx-feature-roadmap.md`, Part 4 unless stated.
- **Effort** S/M/L, **Priority** P1/P2/P3. All proposals are Rust only, GTK4/libadwaita, and respect Part 2:
  D2 SQLite everywhere, D3 PDF deferred, D4 only formats Calibre supports, D5 plugins after Wave 2, D6 TTS
  accepted, D7 CLI/server first-class, D8 e-ink after R13, D9 no LIT.
- The points are ordered by my ranking of frequency and intensity (PP-G-1 is the worst).
- Bookx is a desktop GTK app with a built-in server (`crates/bookx-serve`). Pain points of server-only products
  (users, Docker, NAS) are mapped to what Bookx's Share Library can do.

---

## Summary table

| ID | Pain point | Freq | Bookx | Proposal | Eff | Prio |
|---|---|---|---|---|---|---|
| PP-G-1 | Metadata sources are wrong, rate-limited or die | high | DONE | Provenance, source health, offline cache, review queue | M | P1 |
| PP-G-2 | Edits live in the database, not in the file | high | DONE | Drift check and write-back everywhere | M | P1 |
| PP-G-3 | Series and author naming chaos | high | DONE | Normaliser, series view, author aliases in UI | M | P1 |
| PP-G-4 | Duplicate detection too naive or too late | high | DONE | Numerals, post-import check, edition-aware merge | S | P2 |
| PP-G-5 | Work vs edition vs format is not modelled | medium | DONE (2026-11-20, grouping; shared progress with audiobooks open) | "Work" grouping over several files | L | P3 |
| PP-G-6 | Format fragmentation and device-specific conversion | high | PARTIAL | Send-to-device preflight, real-device test matrix | M | P1 |
| PP-G-7 | DRM and store lock-in | high | DONE | Detect and explain DRM, LCP later | S | P2 |
| PP-G-8 | Reading position sync across apps and devices | high | DONE | Reader as KOSync client, precise matching | M | P1 |
| PP-G-9 | Kobo and Kindle sideloaded books lose sync and notes | high | DONE | Read Kobo DB annotations, carry notes on send | M | P1 |
| PP-G-10 | Highlights are trapped in the app or the store | high | DONE | Wire exporters, add importers | M | P1 |
| PP-G-11 | Device transfer is clumsy | medium | PARTIAL (wireless = KOReader only, D10) | MTP via gio, wireless pull | M | P2 |
| PP-G-12 | Large library performance | high | DONE (2026-11-20; docs/performance.md) | 100k budgets, lazy covers | M | P2 |
| PP-G-13 | Fragile database, cloud sync and schema breaks | high | DONE | Lock guard, schema doc, read-only mode | S | P2 |
| PP-G-14 | Backups are manual or absent | medium | DONE | Scheduler, destinations, restore UI | S | P1 |
| PP-G-15 | Import and migration lock-in | medium | DONE (export to Calibre, 2026-11-20) | Round-trip to Calibre layout, OPF sidecars | M | P2 |
| PP-G-16 | Tracker data: Goodreads API dead, silos | medium | PARTIAL | CSV import/export, Hardcover sync | M | P2 |
| PP-G-17 | Projects get abandoned or change terms | medium | DONE | Documented schema, plain-file export | S | P2 |
| PP-G-18 | Cloud dependence and privacy | medium | DONE | Keep the rule, add a written network ledger | S | P3 |
| PP-G-19 | Accessibility of the reader and the app | medium | PARTIAL | AT-SPI text for the canvas (done), keyboard reading (done), TTS | L | P1 |
| PP-G-20 | Typography and rendering quality | medium | PARTIAL | Knuth-Plass and CSS in the canvas | L | P2 |
| PP-G-21 | CJK, RTL and vertical text | medium | PARTIAL | RTL page direction done; vertical writing planned (`docs/archive/vertical-writing-plan.md`) | L | P2 |
| PP-G-22 | Dictionary and translation | medium | DONE | Fix bug 1, offline-first fallbacks | S | P2 |
| PP-G-23 | E-ink support | medium | PARTIAL | Refresh hints, stay nice-to-have | M | P3 |
| PP-G-24 | Comics and manga | high | PARTIAL (reader DONE 2026-11-20) | next volume in the folder, series from folders, "ignore ComicInfo" switch, real RAR (CBR) | L | P2 |
| PP-G-25 | Audiobook and ebook pairing | medium | PARTIAL (playback DONE 2026-11-20) | one position across e-book and audiobook, EPUB 3 media overlays in the reader, position sync between devices | L | P3 |
| PP-G-26 | Multi-user and sharing | high | PARTIAL | Per-user accounts in Share Library | M | P3 |
| PP-G-27 | Mobile and offline | medium | PARTIAL | Offline-capable web client | L | P3 |
| PP-G-28 | Cover quality | medium | PARTIAL | Candidate picker with resolution score | S | P2 |
| PP-G-29 | Tagging and organisation | medium | DONE | Smart shelves from saved queries | S | P2 |
| PP-G-30 | Search inside books | low | DONE | Keep, add budgets at 100k | S | P3 |
| PP-G-31 | Long-term preservation | medium | PARTIAL | Fixity checksums, validate on import | M | P2 |
| PP-G-32 | Ingest folders are fragile | medium | DONE | Watch folder with a review queue | S | P1 |
| PP-G-33 | Destructive operations without undo | medium | DONE | Operation log with undo | M | P2 |

Counts (recounted from the audit table 2026-11-22, statuses re-checked): **DONE 19, PARTIAL 14, NOT YET 0** (the 2026-11-20 line said 17/16) (33 points; "MOSTLY DONE" is folded into DONE or PARTIAL). Before the audit: DONE 4, MOSTLY DONE 2, PARTIAL 25, NOT YET 2.

## Audit 2026-11-20 (current status; the paragraphs below are the research-time state)

Every point was checked against the code, tests, wiki and UI scenarios on 2026-11-20. **The status column above and
this table are current; the "Bookx:" paragraphs below keep the state at the time of writing and their own dated
updates.** DONE means everything the proposal asks for and that code can solve is built, tested and reachable.

| ID | Status | Evidence | What is left |
|---|---|---|---|
| PP-G-1 metadata sources | DONE | Bulk fetch + review table with source per field, provenance, on-disk cache, health with backoff, fetch for new books, undo incl. covers (`bulk_fetch.rs`, `metadata_fetch.rs`, `docs/metadata-notes.md`); plugin sources | No live-network tests; three built-in sources |
| PP-G-2 edits only in the database | DONE | Every edit goes through `Library::update_metadata` (file and catalogue together); drift check with *use file* / *use catalogue* fixes (`library check --drift`, Health) | AZW3/PDF write-back |
| PP-G-3 series and author chaos | DONE | Series number parser, Series window with gaps, author aliases window, tidy names, `library authors/rename-author` (`series_view.rs`, `authors_view.rs`, `bookx-sources::{authors,aliases,tidy}`) | Language-aware name order; no automatic transliteration |
| PP-G-4 duplicates | DONE | ISBN/title/author fuzzy match, number words, import-time check, merge or merge as formats, **"Not a Duplicate" memory** (`duplicates::Apart`, scenario `pain_apart`) | |
| PP-G-5 work vs edition vs format | DONE (grouping) | One book holds several formats (merge, add format, best format per device); **works above books**: catalogue schema v7 (`work_links`, `work_apart`), pure grouping `src/work_groups.rs` (user links, ISBN / work ids, title+author; translations and other subtitles/numbers/authors never merge), one grid card or table row with an edition badge, edition list (popover and details panel) with Open / Send / Open first / Unlink, *Link as Editions*, *Unlink All Editions*, bulk actions reach every edition, setting *Group editions of the same book* (default off); scenarios `works_*` (Broadway) and `works_editions_*` (real session) | Reading progress is not shared between an EPUB and an audiobook (the card shows the progress of the edition read last; audiobooks are separate books); the Editions & Formats window still lists by title and author only |
| PP-G-6 format fragmentation | PARTIAL | Check Before Sending (validator + Kindle/Kobo/e-mail rules), Fix All, best format per device, conversion report | Check Before Sending now runs in the Device window's send flow (`ui/devices.rs::send_checked`, wave 11); no real-device test matrix (2026-11-22 re-audit) |
| PP-G-7 DRM | DONE | Detection of Adobe, Kindle, Kobo, FairPlay, LCP with a clear message (`core::drm`, `drm:` search) | Synthetic fixtures only; no LCP support (and no circumvention, by design) |
| PP-G-8 position sync | DONE | KOSync client and server, partial-MD5 ids, "continue from X%?" (`kosync_sync.rs`, `ui/kosync.rs`) | Not verified against a real KOReader |
| PP-G-9 Kobo/Kindle notes | DONE | Highlights imported from `KoboReader.sqlite`, KOReader sidecars, My Clippings; notes travel with sent books (`annotation_import.rs`, `send_notes.rs`) | Nothing is written into the Kobo database (by design) |
| PP-G-10 highlights trapped | DONE | Notes & Highlights window, export to Markdown/HTML/JSON/text/Kindle/Readwise/Anki, imports, stable ids | Anki and Readwise only tested against stand-ins |
| PP-G-11 device transfer | PARTIAL | USB mass storage, WebDAV, gvfs mounts, OPDS, e-mail; **wireless sync for KOReader only (owner decision D10, 2026-11-20): KOSync progress + the Bookx plugin for highlights/notes (`docs/koreader-annotation-sync.md`)** | No direct MTP. No wireless sync for stock Kobo/Kindle firmware or other devices by decision (USB only); the Calibre wireless protocol stays deferred. The plugin was never run on a real KOReader |
| PP-G-12 large libraries | PARTIAL | Virtual grid, per-book diff commits, budgets at 10,000 books | **Wave 6:** 100,000-book budgets incl. RSS ceiling, O(n²) fixes, 20k-book Broadway scroll scenario (docs/performance.md); no real-session fps |
| PP-G-13 fragile DB / cloud sync | DONE | Transactions, library lock, read-only mode, network/synced folder warning, **documented schema (wiki "Your data and leaving Bookx")**, `check` | |
| PP-G-14 backups | DONE | Snapshots, restore, off-site with encryption and verification (`docs/backup-notes.md`) | |
| PP-G-15 migration lock-in | DONE (wave 6) | Calibre import (GUI + CLI), JSON dump, save-to-disk templates, citations (BibTeX/RIS/CSL/APA), tracker CSV, annotation exports | Export to a Calibre library with `metadata.db` and per-book `metadata.opf` now exists (`bookx-store::calibre_export`); OPF sidecars next to files in the Bookx library itself are still not written |
| PP-G-16 tracker data | PARTIAL | Goodreads/StoryGraph/Hardcover CSV import, Goodreads/StoryGraph export | Hardcover API client exists (`bookx-sources::hardcover`, read only, stub-tested) but has no screen and applies nothing; Hardcover CSV not checked against a real export (2026-11-22 re-audit) |
| PP-G-17 abandonment / terms | DONE | GPL-3.0-or-later, plain files, SQLite, `library dump`, **documented data layout** | |
| PP-G-18 cloud and privacy | DONE | No telemetry; **list of every connection and an offline mode switch (Preferences → Privacy, wiki Privacy, `src/privacy.rs`)** | |
| PP-G-19 accessibility | PARTIAL | Accessible page text, every control named, keyboard-only reading, Orca, OpenDyslexic, high contrast, TTS, **reading ruler** | Image alt text; popover names not verified |
| PP-G-20 typography | PARTIAL | Hyphenation, Knuth-Plass (setting), highlight styles, user CSS layer, partial book CSS | Wave 11 added widows/orphans, small caps, fonts and sizes from the book CSS, weight/contrast/word spacing/kerning. Still missing: real tables and positioned text, hanging punctuation, `word-break`, emphasis dots (2026-11-22 re-audit) |
| PP-G-21 CJK, RTL, vertical | PARTIAL | RTL pages, vertical writing and furigana in the canvas (`docs/archive/vertical-writing-plan.md`) | Tate-chu-yoko edge cases, selection in vertical text |
| PP-G-22 dictionary / translation | DONE | StarDict with word forms, automatic dictionary download per language, vocabulary with SM-2 and Anki export, Wikipedia with picture, offline-first fallbacks | Translation always goes to an online service (listed in Privacy) |
| PP-G-23 e-ink | PARTIAL | E-ink mode (palette, no animation, wide tap zones) | No frontlight / partial-refresh control; not on hardware |
| PP-G-24 comics | PARTIAL | Comic reader (`ui/reader_comic.rs`, `crates/bookx-comic`): single page, two-page spread (cover alone, wide pages alone), vertical webtoon strip, fit page/width/height/original, manga right to left from ComicInfo or a toggle remembered per book, smart zoom to the panel under a double-click, guided view (Space / Shift+Space), pan, pinch, Ctrl+wheel, thumbnails strip, decoding on worker threads with a memory budget, grey pages and e-ink, ComicInfo page types; defaults in Preferences (`docs/comic-notes.md`) | Wave 11 added next/previous volume, series from folder names, per-library direction and an "ignore ComicInfo" switch, and CBR through the `unrar`/`7z` helper (never tried with a real RAR5 archive). Still missing: panel detection only for plain gutters (no overlapping or slanted panels), not tried on e-ink hardware or with real published comics, no screen-reader text for the pictures |
| PP-G-25 audiobooks | PARTIAL | Pairing of audio files with books; import of folders, zips and M4B as audiobooks of their own; in-app player (chapters, speed 0.5-3x with pitch kept, sleep timer, bookmarks with notes, resume, MPRIS, listening statistics); badge, sidebar entry and `format:` filters (`docs/audiobook-notes.md`) | Wave 11 added a local, manual e-book/audiobook position link (`src/audiobook_link.rs`) and the reader plays a book's own WAV media-overlay narration (`src/overlay_play.rs`). Still missing: position sync between devices, other overlay codecs, no Opus playback, never heard on a real sound card |
| PP-G-26 multi-user | PARTIAL | Share password, Bookx sync accounts, per-user accounts and guest collections for the server (`bookx-serve::accounts`, CLI and settings file only) | No accounts screen in Preferences, no per-user libraries or progress (2026-11-22 re-audit) |
| PP-G-27 mobile / offline | PARTIAL | OPDS, web page, touch controls, PWA web client (`bookx-serve::pwa`, wave 11) | The PWA was never run in a real browser (JS only syntax-checked); no touch hardware test (2026-11-22 re-audit) |
| PP-G-28 cover quality | PARTIAL | **Size and shape grading, small-cover scan, larger covers offered as safe changes in the fetch review (`cover_quality.rs`, Health → Covers)**, cover crop and generation | One candidate per book, no side-by-side picker of several covers |
| PP-G-29 tagging / organisation | DONE | Tags, shelves with smart shelves, saved searches and virtual libraries, query language, tag browser | Hierarchical tags |
| PP-G-30 search inside books | DONE | FTS5 index, `text:` term | Budgets at 100,000 books |
| PP-G-31 preservation | PARTIAL | **SHA-256 fixity with damage detection (`core::fixity`, Health → File integrity, `library fixity`)**, validator, check-before-send | No validate-on-import, no scheduled verification, no keep-original policy in the UI |
| PP-G-32 ingest folders | DONE | Watched import folder with a review queue, files must settle first, remembers what was added or dismissed (`core::inbox`, Health, `library inbox`) | Scans when Health opens and at start-up, not continuously |
| PP-G-33 destructive operations | DONE | Operation log with undo, trash with restore | |

New in this pass (2026-11-20, "pain" track): PP-G-4 (not-a-duplicate), PP-G-13/17 (documented data), PP-G-18 (offline
mode and the list), PP-G-19 (reading ruler), PP-G-28 (cover quality), PP-G-31 (checksums), PP-G-15/17 (citations export).

---

## The points

### PP-G-1 Metadata sources are wrong, rate-limited or die

Automatic metadata is the feature everyone wants and the one that breaks most. Sources get blocked or change
markup, matches are wrong, series and tags go missing, and a single central metadata service can take a whole
product down (Readarr).

- **Update 2026-11-13 (metadata track, `docs/metadata-notes.md`):** still PARTIAL. New: *Fetch metadata for new books* after an import (opt-in, off by default, empty fields only, no window), an on-disk response cache (short TTL per source, size/age capped, hash-keyed files, "not found" kept for an hour, failures never cached), source health that survives restarts, the review table names the source of each field, and undo restores a replaced cover. Not done: live-network tests, a per-source terms review beyond the TTLs, cache use by plugin sources.
- **Update 2026-11-12 (metadata track, `docs/metadata-notes.md`):** still PARTIAL, much further. Done: bulk *Fetch Metadata…* with background lookups, a review table (book, field, current → proposed, source, confidence), accept all / per row / per field / only empty fields, an opt-in "apply safe changes automatically" (empty fields only), per-source health (ok / slow / failing) with exponential backoff, provenance for every applied field (bulk fetch, bulk edit, tidy names, editor metadata tab, identifier cleanup, import patches) and undo through the operation log. Not done: automatic fetch at import, an on-disk response cache that survives an outage, cover proposals are applied but not restored by undo, health is kept for the running session only.
- Evidence: [Readarr retirement and metadata-server notice](https://wiki.servarr.com/readarr/metadata-issues),
  [rapidseedbox on why Readarr ended](https://www.rapidseedbox.com/blog/readarr-alternative),
  [r/Calibre: metadata download suddenly degraded](https://www.reddit.com/r/Calibre/comments/1te1mo7/metadata_download_suddenly_sucks),
  [MobileRead: cover plugins get blocked as DoS](https://www.mobileread.com/forums/showthread.php?t=347944),
  [Audiobookshelf docs: quick-match is not always accurate](https://audiobookshelf.org/docs/documentation/libraries/book-library/book-metadata).
- Frequency: high.
- Bookx: PARTIAL. Fetch Metadata in the edit dialog merges Open Library, Google Books and Wikidata with a
  confidence score (`src/metadata_fetch.rs`, `crates/bookx-sources`, roadmap "Fetch metadata in the edit
  dialog"). Not done: automatic fetch at import, per-field provenance, a cache that survives an outage,
  health display per source.
- Proposal: store provenance per field (source, time, confidence) in the catalogue; keep a local response cache
  so a dead source never blocks editing; show source health in Preferences; add an "Unreviewed matches" queue
  that import fills (no silent overwrite, ever). Plugin sources (D5, already built) cover new providers.
  M, P1.

### PP-G-2 Edits live in the database, not in the file

Many tools change only their own database. The book on the device or in a backup then lacks the corrected
title, series or cover. Others write into the file but only sometimes.

- **Update 2026-11-12 (metadata track, `docs/metadata-notes.md`):** unchanged in substance. Every new edit path above goes through `Library::update_metadata`, which writes the EPUB and the catalogue together, so they stay in step. The editor's metadata tab still saves only when the book is saved.
- Evidence: [Calibre-Web-Automated README: stock Calibre-Web changes only its UI](https://github.com/crocodilestick/calibre-web-automated),
  [CWA #726: series written to OPF but not into the EPUB](https://github.com/crocodilestick/Calibre-Web-Automated/issues/726),
  [r/Calibre: metadata changed to the wrong book](https://www.reddit.com/r/Calibre/comments/1hp6bs1/calibre_keeps_changing_the_metadata_of_books_to).
- Frequency: high.
- Bookx: PARTIAL. `bookx-core::opf` edits metadata inside the EPUB (AGENTS.md lists "opf edits"), and the CLI has
  `meta set`. Not verified: that every catalogue edit path (bulk edit, fetch, merge) writes back, and the roadmap
  lists write-back for AZW3/PDF (S7) as a candidate only.
- Proposal: a "file vs catalogue" drift check (hash plus parsed OPF compare) with a one-click "write catalogue to
  files" and "read files into catalogue"; a per-library choice of which side wins; test that each edit path
  keeps both in sync. M, P1.

### PP-G-3 Series and author naming chaos

Authors appear as "First Last", "Last, First", with initials, with several names in one field, or in
last-name-first languages. Series with volume numbers inside the series name split or merge wrongly.

- **Update 2026-11-12 (metadata track, `docs/metadata-notes.md`):** still PARTIAL. Done: series number parser in the edit dialog (`1`, `01`, `I`, `Book Two` → number, with a sort preview and clash warning), a *Series…* window (order, gaps as "missing #3", read progress, duplicate or missing numbers), series suggestions through the fetch review table. Not done: a language-aware name-order toggle.
- **Update 2026-11-13 (metadata track):** author aliases are built: alias table in the catalogue (schema v6, part of every backup), *Authors…* window grouped by person, merge spellings, rename an author everywhere with a preview and one undo step, suggestions for initials against full names, `bookx-cli library authors|rename-author`. Transliteration is manual (you declare the alias); there is no automatic transliteration table.
- Evidence: [MobileRead: correct author name format](https://www.mobileread.com/forums/showthread.php?t=316012),
  [Calibre bug: Manage Authors inconsistencies, name order](https://bugs.launchpad.net/bugs/1084363),
  [Kavita #1612: books with two authors vanish from one series](https://github.com/Kareadita/Kavita/issues/1612),
  [Kavita 0.8 notes: epubs with a volume inside the series name](https://github.com/Kareadita/Kavita/discussions/2851),
  [Kavita wiki: comic trades become separate series](https://wiki.kavitareader.com/guides/metadata/comics).
- Frequency: high.
- Bookx: PARTIAL. `crates/bookx-sources/src/authors.rs` has name parsing, `author_sort`, `split_authors`,
  `same_author` (roadmap L34); the edit dialog has Series and Series number (`src/ui/metadata.rs`). Not seen in
  the app: a series grouping view, an author manager with aliases, cleanup of sort names in bulk.
- Proposal: "Tidy names" bulk action (preview list, apply) built on `authors.rs`; a Series view that groups by
  series and flags gaps and duplicates in the number sequence; author aliases that survive fetches; a
  language-aware name-order toggle. M, P1.

### PP-G-4 Duplicate detection is too naive or too late

Exact-match scans miss "Eclipse 2" vs "Eclipse two" and variant author forms. Others only warn after a duplicate
was already imported and cross-linked.

- **Update 2026-11-12 (metadata track, `docs/metadata-notes.md`):** *Editions & Formats…* groups books with the same normalised title and author as one work (view only). The catalogue has no multi-format record (every import becomes one EPUB), so "merge as formats of one book" was not built; Find Duplicates stays the (destructive, confirmed) merge. Number-word matching and the import-time check are untouched.
- Evidence: [CWA 4.0 thread: scan misses "Eclipse 2" vs "Eclipse two"](https://www.reddit.com/r/selfhosted/comments/1qqb2pd/calibreweb_automated_v400_released_smart),
  [r/Calibre: duplicates from author sort issues](https://www.reddit.com/r/Calibre/comments/1j04alt/duplicates_through_author_sort_issues),
  [CWA README: duplicate import and corrupt DB when ingest is misused](https://github.com/crocodilestick/calibre-web-automated),
  [Kavita: one manga split in two by ComicInfo](https://github.com/Kareadita/Kavita/discussions/2854).
- Frequency: high.
- Bookx: PARTIAL. Fuzzy detection exists: ISBN, normalised titles (accents, articles, subtitles), author forms,
  initials (`crates/bookx-sources/src/dedup.rs`), a keep/merge UI that unions notes and progress
  (`src/duplicates.rs`, threshold 0.9, roadmap "Duplicate finder"). Gaps: I did not find number-word handling
  ("two" vs "2"), a check at import time, or a "not a duplicate" memory.
- Proposal: add numeral/roman-numeral normalisation with a test for the CWA example; run the finder on the new
  books right after import and show a toast with "Review"; remember "not duplicates" pairs in SQLite. S, P2.

### PP-G-5 Work vs edition vs format is not modelled

Readers think "a book"; catalogues hold editions, translations, and several files per book. Trackers drown in
duplicate editions; managers cannot attach an ebook and an audiobook to the same title cleanly.

- Evidence: [StoryGraph: merge editions request](https://roadmap.thestorygraph.com/requests-ideas/posts/merge-editions),
  [StoryGraph: reading any edition should clear To Read](https://roadmap.thestorygraph.com/requests-ideas/posts/to-read-accepts-any-edition-as-read),
  [Goodreads librarian manual: combining editions](https://help.goodreads.com/s/article/Librarian-Manual-How-to-combine-editions),
  [Audiobookshelf: only the primary ebook keeps progress](https://audiobookshelf.org/docs/documentation/libraries/book-library/ebooks).
- Frequency: medium.
- Bookx: PARTIAL (update 2026-11-13). A book now holds several format files (catalogue schema v6, `core::merge`):
  *Merge as formats of one book* with preview and undo, *Add as another format* at import, formats listed in the details panel, the
  best format per purpose (reader, Kobo, Kindle, e-mail, conversion), Find Duplicates offers *Merge Formats* when the copies differ in
  format, `bookx-cli library formats|add-format|remove-format|merge-formats`. Still missing: editions as a level above books ("work" ids;
  *Editions & Formats* is still a grouping view by title and author), reading progress shared between an EPUB and an audiobook.
  (Earlier: no multi-file concept; merge in `src/duplicates.rs` deleted the other file.)
- **Update 2026-11-20 (works grouping):** done as a *view* over books, never a silent merge. Schema v7 adds `work_links`
  (book → work id, plus the edition that opens first) and `work_apart` (books the user took out of the automatic
  grouping). The automatic suggestions are computed from the books each time (`work_groups::group`, ISBN-13 and work
  identifiers, then the normalised main title with a compatible subtitle, same author, compatible language) and never
  stored. A book without a language only goes with books of one language; a pair with two languages in sight (a
  translation) stays apart. Choosing the edition that opens first on an automatic group turns the group into a link
  (the choice must survive new imports). Default of the setting: **off**, because it hides books in the grid and the
  existing counts and tile tests assume one card per book; *Link as Editions* turns it on. Perf: grouping 20 000 books
  (a fifth of them editions) is a unit test with a 400 ms release budget (measured about 0.1 s including building
  the books). Not done: sharing reading progress between an EPUB and an audiobook (PP-G-25 attaches audio as a
  format/resource of a book, not as an edition), the Series window does not group editions, the OPDS/content server
  and devices still list books, not works.
- Proposal: a "work" id in SQLite that groups several files (EPUB, KEPUB, audiobook) and editions; progress and
  highlights attach to the work; the duplicate finder offers "group" as an alternative to "merge and delete".
  L, P3 (schema decision first).

### PP-G-6 Format fragmentation and device-specific conversion

Kobo wants KEPUB for chapter progress and highlights; Amazon rewrites EPUB into its own formats and rejects some
EPUBs; every device has quirks. Users end up with copies in several formats.

- **Update 2026-11-12 (metadata track, `docs/metadata-notes.md`):** still PARTIAL. Done: *Check Before Sending…* (validator plus Kindle, Kobo, e-mail rules: language, DRM, 25 MB mail limit, cover, author), one-click *Fix All* for what `bookx_editing::fixes` repairs (originals kept), an optional check step in *Convert To* and *Send by E-mail*, and a conversion report next to the output. Update 2026-11-13: a book with several formats sends the one that suits the device (KEPUB to a Kobo, AZW3/MOBI to a Kindle, EPUB otherwise). Not done: the check in the Device window's send flow, the real-device test matrix, a full Amazon rule set beyond the above.
- Evidence: [calibre-web #1484: only KEPUB keeps annotations and chapter progress on Kobo](https://github.com/janeczku/calibre-web/issues/1484),
  [CWA 4.0: EPUB fixer for Amazon rejections (E999)](https://www.reddit.com/r/selfhosted/comments/1qqb2pd/calibreweb_automated_v400_released_smart),
  [Amazon changes to Send to Kindle conversions](https://www.rsinc.com/how-to-remove-drm-from-kindle-books-in-2025.php),
  [KOReader hyphen differences between store and converted AZW3](https://www.answeroverflow.com/m/1400917710703038485).
- Frequency: high.
- Bookx: PARTIAL. Conversion is broad (EPUB, KEPUB, AZW3, MOBI, FB2 and more, checked against Calibre as an
  oracle, `crates/bookx-convert`), Send to device picks KEPUB/AZW3/EPUB (`src/devices.rs`), and an 87-rule
  validator exists (`bookx_editing::validate`). Roadmap says the Kindle outputs were "NOT tried on a real
  Kindle".
- Proposal: run the validator plus an "Amazon-safe" rule set (language tag, XML declaration, CSS) as a preflight
  before Send by E-mail and Send to device, with a one-click fix; publish a device test matrix (real Kindle,
  Kobo, PocketBook) with golden files; keep D4 (no formats Calibre lacks). M, P1.

### PP-G-7 DRM and store lock-in

Purchased books are tied to a store account. Amazon removed Download and Transfer via USB in February 2025, and
format changes (KFX) make later conversion harder. Open readers can only help for open or LCP-protected books.

- Evidence: [Amazon ends Download and Transfer, HN discussion](https://news.ycombinator.com/item?id=43041726),
  [The eBook Reader blog on the change](https://blog.the-ebook-reader.com/2025/02/12/download-transfer-for-kindle-ebooks-going-away-on-february-26),
  [AkitaOnRails: DRM removal in 2026, formats and remote changes](https://www.akitaonrails.com/en/2026/07/30/removing-drm-from-kindle-ebooks-in-2026),
  [Thorium and LCP for library loans (Duke)](https://blogs.library.duke.edu/blog/2026/05/04/thorium-reader-for-ebsco-ebooks).
- Frequency: high.
- Bookx: PARTIAL (update 2026-11-13). Detection and a clear message are built (`core::drm`): Adobe ADEPT, Kindle (MOBI/AZW encryption
  flag, KFX DRM header), Kobo KEPUB, Apple FairPlay, Readium LCP, by structural markers only; a protected EPUB is listed with a "Protected (…)"
  notice and refused by the reader, other protected files are refused at import, `drm:` finds them. Tested with synthetic fixtures only, not
  with real purchased files. Bookx does not remove DRM and has no LCP support. (Earlier: not in the roadmap, no detection in the importer.) Legal position: Bookx should
  not ship circumvention (not stated by the owner; flagged in Unverified).
- Proposal: detect Adobe/Amazon/Kobo DRM and LCP on import and say so plainly ("this file is protected, here is
  what you can do") instead of a parse error; recognise `.lcpl`; consider Readium LCP support only if a Rust
  implementation is licence-compatible (decision for the owner). S, P2.

### PP-G-8 Reading position sync across apps and devices

There is no universal standard. KOSync identifies books by a file hash, so a changed file breaks matching; some
clients push but never pull; some clients write only a rough position; Kobo's own sync was unreliable until a
rebuild.

- Evidence: [MobileRead: sync works Kobo-to-Kobo but not Kobo-to-Kindle; hash mismatch](https://www.mobileread.com/forums/showthread.php?t=320017),
  [Readest discussion: push works, pull does not](https://github.com/readest/readest/discussions/1838),
  [crosspoint-reader #2067: one-way precision](https://github.com/crosspoint-reader/crosspoint-reader/issues/2067),
  [Readest proposal: KOSync progress mismatch with Kavita](https://github.com/readest/readest/discussions/4258),
  [Kobo rebuilds its progress sync](https://www.makeuseof.com/kobo-finally-solves-its-infuriating-syncing-problem-heres-how-to-fix-it),
  [calibre-web #2122: no standard for progress sync](https://github.com/janeczku/calibre-web/issues/2122).
  Calibre 9.18 added a KOReader-compatible sync endpoint (see its [release video summary](https://www.youtube.com/watch?v=WHQld_h-X34); unverified against the changelog).
- Frequency: high.
- Bookx (update 2026-11-12): DONE in code, see `docs/annotation-sync-notes.md`. The reader is now a KOSync client
  (Preferences → Sync reading position: server, account, Register/Log in, partial-MD5 or file-name ids, pull with
  "Continue from X at 63%?" on open, debounced push, hold-back instead of moving the server backwards) and it is
  tested against Bookx's own server in-process and in a UI scenario with a second client. NOT verified against a
  real KOReader or sync.koreader.rocks; the position is percentage-based (documented limits). Original analysis follows.
- Bookx (before): PARTIAL. A KOSync-compatible server exists with tests (`crates/bookx-serve/src/kosync.rs`) and is a
  switch in Share Library (`src/ui/share.rs`, `src/sharing.rs`). As far as I could see, the reader does not
  read or write that store for local books, and the server stores opaque document ids without mapping them to
  catalogue books or exposing the progress to the reader.
- Proposal: compute KOReader's partial-MD5 ("binary") and filename ids for every book; the reader pushes on
  close and page-turn bursts, pulls on open and shows "Continue from X% on Kobo?"; resolve conflicts by
  timestamp and furthest-read with a visible choice; keep a position model that stores both percent and
  chapter+offset so precision survives clients that send only a percent. Add a Bookx-as-client mode for a remote
  KOSync server. M, P1.

### PP-G-9 Kobo and Kindle sideloaded books lose sync and notes

Sideloaded books on Kobo do not sync to the cloud, highlights vanish after firmware updates, and notes on
transferred books do not travel back.

- Evidence: [Kobo support: missing sideloaded highlights after a software update](https://help.kobo.com/hc/en-us/articles/43753873356055-Missing-books-reading-stats-or-notes-after-Kobo-software-update),
  [r/kobo: no progress sync for sideloaded content](https://www.reddit.com/r/kobo/comments/1pr0akf/a_rant_about_the_missing_reading_progress_sync),
  [calibre-web #1484](https://github.com/janeczku/calibre-web/issues/1484),
  [Facebook reading nook: 1000+ sideloaded books cannot sync](https://www.facebook.com/groups/thereadingnookonfb/posts/827816759934923).
- Frequency: high.
- Bookx (update 2026-11-12): MOSTLY DONE. Highlights and notes are imported read-only from `KoboReader.sqlite` (a copy of
  it, or a mounted Kobo) and KOReader sidecars, matched by title/author, reviewed before applying (Import Highlights…);
  the USB sync also reads them. Still open: writing highlights into the Kobo database/KEPUB annotation store (we only
  write KOReader sidecars, never the Kobo DB), Kindle's own annotation state (only `My Clippings.txt` and KOReader
  files are readable). Update 2026-11-13: a mounted Kindle's `documents/My Clippings.txt` is read-only (test proves
  nothing on the Kindle is touched); the Kindle's binary `.sdr` / `.azw3r` / `.mbp` files are deliberately not parsed
  (undocumented format, no way to verify). Original analysis follows.
- Bookx (before): PARTIAL. USB send to Kobo with KEPUB conversion exists (`src/devices.rs`); the code can locate
  `.kobo/KoboReader.sqlite` (`crates/bookx-serve/src/devices.rs`), but writing the Kobo DB is a roadmap gap (P2/P14)
  and `docs/bugs.md` #2 says Send to device does not carry annotations (pending, p1).
- Proposal: first read-only: import highlights, notes and reading state from `KoboReader.sqlite` (rusqlite,
  allowed by D2) into the catalogue; then fix bug 2 by writing highlights as KOReader-style sidecar or into the
  KEPUB annotation store when safe; always copy the DB before touching it. M, P1.

### PP-G-10 Highlights are trapped in the app or the store

Kindle personal documents do not sync highlights to the cloud, store exports are per book or purchased only, and
Kobo exports need a config edit or a cloud round trip. People want Markdown, Obsidian, Readwise, Anki.

- Evidence: [Export Kindle highlights, limits for personal documents](https://textmuncher.com/blog/export-kindle-highlights-notes),
  [Kobo help: export only for purchased books, files kept 30 days](https://help.kobo.com/hc/en-us/articles/29991333812631-Export-annotations-from-your-books),
  [Goodreads help: Kindle export as PDF or flashcards](https://help.goodreads.com/s/article/000001752),
  [Readest KOReader plugin gains two-way annotation sync](https://www.ereadersforum.com/threads/koreader-sync-and-readest-cross-device-progress-sync-on-the-xteink-x4-ios-and-android.13434).
- Frequency: high.
- Bookx (update 2026-11-12): MOSTLY DONE. Export Annotations now offers Markdown, HTML, JSON, text, Obsidian, Kindle
  clippings and the Readwise API body, per book and for the whole library (Export All Annotations…); Import
  Highlights… reads My Clippings.txt, KOReader sidecars/JSON/Markdown, Kobo DB and calibre exports. Still open: Anki
  export, a direct Readwise upload (we write the JSON body; no token handling), stable highlight ids across re-exports,
  a cross-book Notes view. Update 2026-11-13: all four done (Notes & Highlights window, content-derived ids in exports
  and imports, Anki `.apkg` / TSV / CSV with cloze and basic cards from highlights and vocabulary, Readwise upload with a
  keyring token and an incremental ledger). The `.apkg` and the Readwise request were tested against local stand-ins
  only, not Anki or readwise.io. Original analysis follows.
- Bookx (before): PARTIAL. The app exports JSON, Markdown and text; `crates/bookx-lookup/src/export.rs` also has
  Kindle clippings and Readwise but they are not wired into the UI (grep of `src/` finds no use). Importers
  (My Clippings.txt, KOReader sidecar, Kobo DB) are NOT YET.
- Proposal: wire the existing exporters as targets in the notes view (including Obsidian and Anki, roadmap
  R32); add importers for My Clippings.txt and Kobo; a cross-book Notes view (roadmap idea "commonplace book")
  with stable highlight ids so re-export does not duplicate. M, P1.

### PP-G-11 Device transfer is clumsy

Users need cables, wired mounts, MTP quirks and wireless protocols that are hard to set up.

- Evidence: [MobileRead: wireless device connection to Calibre in a container does not work](https://www.mobileread.com/forums//showthread.php?t=355952),
  [eol's blog: manual sync gluing KOReader, Calibre-Web and kosync](https://blog.sysco.re/blog/reading_system),
  [Amazon's removal of USB download](https://blog.the-ebook-reader.com/2025/02/12/download-transfer-for-kindle-ebooks-going-away-on-february-26),
  [Syncthing-based setup as a workaround](https://medium.com/@philwornath/self-hosted-e-book-sync-between-android-kindle-and-koreader-reclaim-your-reading-be5f5c69e2f3).
- Frequency: medium.
- Bookx: PARTIAL. USB mass storage plus OPDS and web page via Share Library (roadmap P1, P4, P10); WebDAV and gvfs
  MTP mounts. **Owner decision D10: wireless sync is supported only for KOReader** (progress through KOSync, highlights and
  notes through the Bookx plugin); stock Kobo/Kindle firmware and other devices are USB only, so the Calibre wireless protocol
  stays deferred (P5).
- Proposal: MTP through GVfs/`gio` (already a GTK dependency, no libmtp) for Android phones and some readers; a
  "scan this QR code" flow for the OPDS page (roadmap P20); WebDAV as a folder source. M, P2.

### PP-G-12 Large library performance

Startup, add and search get slow beyond tens of thousands of books; memory grows; import over a network share is
painful.

- Evidence: [Calibre taking 30+ minutes to start with 20K books](https://www.mobileread.com/forums/showthread.php?t=370839),
  [r/Calibre: handling about 700k books](https://www.reddit.com/r/Calibre/comments/16z1rgt/my_experience_with_handling_about_700k_books),
  [CWA 4.0: faster search and thumbnails for 50k+ libraries](https://www.reddit.com/r/selfhosted/comments/1qqb2pd/calibreweb_automated_v400_released_smart),
  [Calibre bug: search extremely slow](https://bugs.launchpad.net/bugs/1862972).
- Frequency: high.
- Bookx: PARTIAL. SQLite catalogue with per-book diff commits and perf budgets in `crates/*/tests/perf_budgets.rs`
  (first save of 10,000 books, one edit among 10,000, reopen; roadmap Part 4). Roadmap also notes reader/scroll
  budgets are TODO. Nothing tests 100k books.
- Proposal: add 100k-book budgets (open, search, FTS build, memory ceiling), virtualise and lazy-load covers in
  the library grid (the "scroll 10,000 covers at 60 fps" idea in Part 1), and rate-limit the full-text indexer so
  the UI stays responsive. M, P2.

### PP-G-13 Fragile database, cloud sync and schema breaks

Library databases sit in sync folders and on network shares, get corrupted by concurrent writers, and break
third-party tools when the schema changes.

- Evidence: [Calibre 9 changed metadata.db and broke calibre-web](https://davidroessli.com/logs/calibre-9-broke-my-calibre-web-server),
  [CWA #967: Calibre 9 incompatibility](https://github.com/crocodilestick/Calibre-Web-Automated/issues/967),
  [CWA 4.0: NFS/SMB locking mode to avoid corruption](https://www.reddit.com/r/selfhosted/comments/1qqb2pd/calibreweb_automated_v400_released_smart),
  [LinuxServer forum: calibre-web does not notice metadata.db changes via cloud sync](https://discourse.linuxserver.io/t/calibre-web-is-not-updating-library-when-metadata-db-changes/1668).
- Frequency: high.
- Bookx: PARTIAL. Atomic transaction commits and a `CatalogWatcher` on `PRAGMA data_version` for the server
  (`crates/bookx-store`, roadmap "SQLite is the catalogue"). Not seen: detection of a network or synced folder,
  a single-writer lock with a clear message, a documented stable schema for third-party tools.
- Proposal: an advisory lock file plus "this library is open elsewhere" message; warn when the library folder is
  on NFS/SMB/cloud-sync; publish the schema and version in `docs/` and keep `library dump` as the stable
  interface. S, P2.

### PP-G-14 Backups are manual or absent

Most tools tell users to copy the folder. Restores need the right tool version.

- Evidence: [Calibre FAQ: a library is just a folder](https://manual.calibre-ebook.com/faq.html),
  [Liber: back up before migrating](https://tryliber.com/blog/switch-from-calibre),
  [note.com: moving a Calibre library, what to copy](https://note.com/yaminabei/n/n07bcf157f4aa?hl=en),
  [Booklore shutdown: users had no easy export](https://www.xda-developers.com/single-maintainer-open-source-ticking-time-bomb).
- Frequency: medium.
- Bookx: DONE since 2026-11-12: automatic snapshots, Back up now, restore, and copies to drives, shares, cloud-drive mounts and
  WebDAV with optional encryption (`docs/backup-notes.md`). Earlier status: PARTIAL, no caller of `backup_to`.
- Proposal: Preferences → Library → Backups: schedule, keep N, location (including an external drive), "Restore
  from backup" and "Check library" actions, a toast on failure; test restore end to end with the UI suite. S, P1.

### PP-G-15 Import and migration lock-in

Moving between managers loses series, custom columns, read status or folder layout. Calibre rewrites the folder
structure. Other tools ship Calibre importers as their main selling point.

- Evidence: [Alfa Ebooks Manager: Calibre moves your files into its own folders](https://www.alfaebooks.com/best_calibre_alternative),
  [Liber: switch from Calibre via OPF](https://tryliber.com/blog/switch-from-calibre),
  [Calibre FAQ: use Save to disk if development ever stops](https://manual.calibre-ebook.com/faq.html),
  [CWA #388: breakage migrating from calibre-web](https://github.com/crocodilestick/Calibre-Web-Automated/issues/388).
- Frequency: medium.
- Bookx: PARTIAL. Calibre `metadata.db` import exists (`crates/bookx-store/src/calibre.rs`, roadmap L10); the
  roadmap mentions "export back" but I did not verify it. `Save to disk` with templates is a gap (template
  language exists in `bookx-sources`, no UI).
- Proposal: "Export library" to a Calibre-compatible layout and to plain folders via a template; a mode that
  indexes files in place without moving them; import from other managers (Kavita/Komga series folders, CWA
  library). M, P2.

### PP-G-16 Tracker data: Goodreads API dead, silos

Goodreads closed its API; tracking data is hard to export; alternatives have their own gaps. Self-hosters want
progress mirrored to a tracker.

- Evidence: [Hardcover's API as a Goodreads API alternative](https://www.emgoto.com/hardcover-book-api),
  [Goodreads alternatives and CSV import paths](https://www.achriom.com/blog/best-goodreads-alternatives),
  [Komga community: Seenr marks finished books in a tracker](https://komga.org/docs/community),
  [Kavita scrobbling notes](https://github.com/Kareadita/Kavita/discussions/2851).
- Frequency: medium.
- Bookx: PARTIAL (update 2026-11-13). *Reading Trackers…* imports Goodreads/StoryGraph/Hardcover CSV (rating, status, date read, shelves as
  tags; match by ISBN then title and author; preview; ratings and tags one undo step) and exports the Goodreads and StoryGraph layouts
  (`bookx_sources::trackers`, `src/tracker_sync.rs`). Hardcover columns are matched by name and were not checked against a real Hardcover
  export. No Hardcover API sync. Reading statistics are local (`src/reading_stats.rs`).
- Proposal: import Goodreads/StoryGraph CSV (read status, dates, ratings, shelves); export CSV; optional
  Hardcover sync through its GraphQL API behind an explicit opt-in toggle and a visible network ledger (see
  PP-G-18). M, P2.

### PP-G-17 Projects get abandoned or change terms

Single-maintainer apps vanish; a retired metadata service ends a product; licence or permission changes
surprise users.

- Evidence: [XDA on Booklore disappearing, Grimmory fork](https://www.xda-developers.com/single-maintainer-open-source-ticking-time-bomb),
  [Notebookcheck: Booklore licensing controversy and changed file permissions](https://www.notebookcheck.net/Booklore-The-Plex-of-self-hosted-e-book-libraries-sparks-licensing-controversy-and-community-backlash.1287933.0.html),
  [Readarr retirement notice](https://readarr.com),
  [DietPi forum on Readarr alternatives](https://dietpi.com/forum/t/readarr-has-been-retired-what-about-lazylibrarian-instead/24105).
- Frequency: medium.
- Bookx: PARTIAL. GPL-3.0-or-later, books stay as normal files, catalogue is SQLite, `library dump` gives full
  JSON, product principle X15 (plain, portable data folder).
- Proposal: document the on-disk layout and schema; add "export sidecars" (OPF per book) so any tool can rebuild
  the library; never change file permissions or ownership of user files. S, P2.

### PP-G-18 Cloud dependence and privacy

Store features depend on an account and a server (cloud-only exports, accounts for sync). Analytics sneak into
self-hosted tools.

- Evidence: [Kobo export works only through the cloud, purchased books only](https://help.kobo.com/hc/en-us/articles/29991333812631-Export-annotations-from-your-books),
  [Readwise syncs only store-bought Kindle books](https://textmuncher.com/blog/export-kindle-highlights-notes),
  [DietPi forum: Servarr analytics objection](https://dietpi.com/forum/t/readarr-has-been-retired-what-about-lazylibrarian-instead/24105).
- Frequency: medium (only the third link is about telemetry; see Unverified).
- Bookx: DONE. No telemetry or analytics code found (grep for "telemetry" in `crates/` and `src/` is empty);
  roadmap X15 sets the rule; network access goes through a `Fetcher` trait (`crates/bookx-sources`), and the
  server needs a password for LAN (roadmap "Share Library").
- Proposal: keep it that way; list every outbound host in Preferences ("What Bookx connects to") and in docs, so
  users and packagers can verify. S, P3.

### PP-G-19 Accessibility of the reader and the app

EPUB accessibility needs screen reader support, keyboard operation, adjustable text and spacing, and readable
alternatives. Custom-drawn reading surfaces tend to be invisible to assistive tech.

- Evidence: [DAISY Consortium: Thorium getting started (screen readers, keyboard, TTS)](https://daisy.org/guidance/info-help/guidance-training/reading-systems/thorium-epub-reader-quick-start-guide),
  [UC Riverside SDRC on Thorium](https://sdrc.ucr.edu/thorium),
  [talk on Thorium accessibility testing: spacing, contrast, read aloud](https://www.youtube.com/watch?v=aYTnW1WLqKs),
  [Calibre 9.18 adds EPUB accessibility tools](https://www.youtube.com/watch?v=WHQld_h-X34).
- Frequency: medium.
- Bookx: PARTIAL (see `docs/a11y-notes.md`). The canvas now exposes the visible page through `GtkAccessibleText`, every control
  is named (audited), the reader works by keyboard only, Orca was run against it. Old state: the book is drawn by a custom canvas (`src/ui/canvas.rs`); there was no
  accessible role/label code anywhere in `src/`. The editor can make EPUBs accessible (`src/editor_tools.rs`,
  `bookx_editing::accessibility`). TTS (R10, D6) is accepted but not built; dyslexia fonts R64 not done.
- Proposal: expose the visible page text to AT-SPI (a hidden accessible text role or a `gtk::AccessibleText`
  implementation when available in the gtk4-rs version in use), keyboard-only reading and link following (R43),
  proper labels on all controls, speech-dispatcher TTS with sentence highlight, bundled dyslexia-friendly fonts
  listed in `NOTICE.md`; add a UI scenario that walks the app with the keyboard. L, P1.

### PP-G-20 Typography and rendering quality

Readers differ widely in justification, hyphenation, margins and how much of the book's CSS they honour. Users
choose KOReader mainly for rendering.

- Evidence: [HN: KOReader as best renderer and interface](https://news.ycombinator.com/item?id=43600991),
  [Kindle store books are hyphenated, converted ones are not (KOReader discussion)](https://www.answeroverflow.com/m/1400917710703038485),
  [KOReader README: typesetting options and bundled hyphenation](https://github.com/koreader/koreader),
  [Mashable e-reader roundup: Boox lag with some apps](https://mashable.com/roundup/best-e-readers-expert-tested-2026).
- Frequency: medium.
- Bookx: PARTIAL. Hyphenation, highlight styles, partial book CSS (indents, margins, small caps, etc.) are in the
  canvas; Knuth-Plass, colours, fonts, floats, drop caps and the user CSS override layer are open (roadmap
  "Not yet integrated").
- Proposal: finish the CSS properties list, wire `bookx-text` Knuth-Plass behind a setting, add widow/orphan
  control, and a typography profile UI (R19/R34). L, P2.

### PP-G-21 CJK, RTL and vertical text

Vertical Japanese, right-to-left layout and punctuation shaping break in web-based readers.

- Evidence: [Foliate #1519: Japanese brackets and punctuation in vertical text](https://github.com/johnfactotum/foliate/issues/1519),
  [CWA #678: vertical RTL EPUBs shown horizontally](https://github.com/crocodilestick/Calibre-Web-Automated/issues/678).
- Frequency: medium (two sources found; likely more in language-specific forums I did not reach).
- Bookx: PARTIAL. RTL: `page-progression-direction="rtl"` in the spine is read (`Package::rtl`) and mirrors the arrow keys, tap zones and
  swipes; paragraphs are already laid out right to left. Vertical writing is NOT YET in the canvas (plan: `docs/archive/vertical-writing-plan.md`, roadmap
  "Known gaps": R4). The RTL flag is also parsed for comics in `bookx-formats`.
- Proposal: use Pango vertical gravity and bidi paragraph direction in the canvas; honour `writing-mode` and
  `page-progression-direction`; ruby via Pango attributes (R56); include a CJK/Arabic sample set in the UI test
  fixtures. L, P2.

### PP-G-22 Dictionary and translation

Offline dictionaries, word lists and translation quality are uneven; vocabulary lists disappear on firmware
updates.

- Evidence: [Kobo software update note: My Words list restored](https://help.kobo.com/hc/en-us/articles/43753873356055-Missing-books-reading-stats-or-notes-after-Kobo-software-update),
  [KOReader README: StarDict and Wikipedia lookup](https://github.com/koreader/koreader),
  [Readest/KOReader ecosystem notes on annotation sync](https://www.ereadersforum.com/threads/koreader-sync-and-readest-cross-device-progress-sync-on-the-xteink-x4-ios-and-android.13434).
- Frequency: medium.
- Bookx: PARTIAL. StarDict offline lookup with word forms (`src/dictionaries.rs`), vocabulary builder with SM-2
  (`src/vocabulary.rs`). `docs/bugs.md` #1 says Wikipedia/dictionary search in the reader is not working
  (pending, p2).
- Proposal: fix bug 1 with a UI scenario; ship a "get a dictionary" helper that lists known free StarDict
  sources (licence checked); export vocabulary to Anki (CSV); keep lookups local-first and make network
  fallbacks explicit. S, P2.

### PP-G-23 E-ink support

E-ink devices need no animation, high contrast and refresh control. Third-party apps lag on some devices.

- Evidence: [KOReader README: e-ink optimised UI](https://github.com/koreader/koreader),
  [Mashable: Boox Go 7 lags with library apps](https://mashable.com/roundup/best-e-readers-expert-tested-2026),
  [BookShelves vs KOReader comparison of e-ink features](https://getbookshelves.app/alternatives/koreader).
- Frequency: medium for readers in general; low for a desktop GTK app.
- Bookx: PARTIAL. E-ink mode exists (`src/eink.rs`: black on white, no animations, wide tap zones), with no
  frontlight or partial-refresh control (roadmap). Owner decision D8: nice-to-have.
- Proposal: keep it nice-to-have; add a "full refresh every N pages" hint hook and a contrast slider; do not
  invest more until a real device is tried (roadmap says touch was not tried on a real screen). M, P3.

### PP-G-24 Comics and manga

Series detection from folders, ComicInfo conflicts, per-library reading direction and double-page handling.

- Evidence: [Kavita: default reading direction per library, manual change each time](https://github.com/Kareadita/Kavita/discussions/2537),
  [Kavita: ComicInfo split one manga into two](https://github.com/Kareadita/Kavita/discussions/2854),
  [Komga #1372: series metadata from ComicInfo not planned](https://github.com/gotson/komga/issues/1372),
  [Kavita wiki on comic metadata and trades](https://wiki.kavitareader.com/guides/metadata/comics),
  [BookOrbit: folder-as-series for comics](https://github.com/bookorbit/bookorbit/issues/1167).
- Frequency: high (in the comic community).
- Bookx: PARTIAL (update 2026-11-20: the page reader is built). Readers for CBZ, CBT, CBC with RTL flag
  (`crates/bookx-formats/src/comic.rs`, roadmap C2, R17/R52) and since this update a comic reader with single page, two-page
  spreads, webtoon strip, panel zoom and guided view, direction and layout remembered per book (`docs/comic-notes.md`).
  Still open: per-library direction, "ignore ComicInfo for this library", folder-as-series, next volume.
- Proposal: a dedicated image reader mode (single, double, webtoon) with per-book and per-collection
  direction remembered, ComicInfo `Manga` honoured, "ignore ComicInfo for this library" switch, folder-as-series
  option. L, P2.

### PP-G-25 Audiobook and ebook pairing

Users own both formats and want one entry, one progress and a seamless switch. Servers disagree on scope.

- Evidence: [audiobookshelf-app #1830: sync ebook and audiobook position](https://github.com/advplyr/audiobookshelf-app/issues/1830),
  [audiobookshelf #3084: EPUB 3 media overlays](https://github.com/advplyr/audiobookshelf/issues/3084),
  [Audiobookshelf docs: progress stored only for the primary ebook](https://audiobookshelf.org/docs/documentation/libraries/book-library/ebooks),
  [Readarr successor Chaptarr handles both media types](https://docs.elfhosted.com/app/readarr).
- Frequency: medium.
- Bookx: PARTIAL (update 2026-11-20: audiobooks play in Bookx, see `docs/audiobook-notes.md`; the text below is the state of 2026-11-13). Audiobook pairing is built: `.m4b` files and folders of audio parts are found next to the books or in a
  folder you name, paired by folder, ISBN, title and author, linked to the book as an `audiobook` resource, shown in the details panel with
  Open (system player) and Unlink. No playback, no position sync, no EPUB 3 media overlays (roadmap R18 stays "low priority").
- Proposal: step 1 EPUB 3 media overlays (read-along) as R18; step 2 attach audio files to a work (needs PP-G-5)
  and map positions through overlays only; do not promise forced alignment. L, P3.

### PP-G-26 Multi-user and sharing

Families need separate progress, shelves and age limits; servers differ on how complete this is.

- Evidence: [Kavita vs Calibre-Web comparison: per-user progress, web reader only](https://dev.to/selfhostingsh/kavita-vs-calibre-web-which-should-you-self-host-3p04),
  [Komga docs: KOReader sync needs a role, API keys per device](https://komga.org/docs/guides/koreader),
  [CWA/Kavita feature lists: age limits, OIDC](https://repocloud.io/details/Calibre-Web%20Automated),
  [Kavita feature list](https://repocloud.io/details/Kavita).
- Frequency: high (self-hosted users).
- Bookx: PARTIAL. Share Library has one password for LAN, optional KOSync users (`src/sharing.rs`,
  `crates/bookx-serve`). Per-user libraries, shelves and progress are not there.
- Proposal: a small accounts table in the server database (per-user progress, collections, read status);
  per-device API keys like Komga; keep it optional so single-user desktop use stays simple. M, P3.

### PP-G-27 Mobile and offline

People want the library on a phone and offline reading without mirroring everything.

- Evidence: [Readest proposal: fetch on demand instead of a full mirror](https://github.com/readest/readest/discussions/4258),
  [Calibre Sync FAQ: cloud libraries, downloaded books](https://calibresync.bitbucket.io/faqs),
  [Komga lists many third-party mobile clients](https://komga.org/docs/category/readers),
  [Kavita advertises PWA](https://jellywatch.app/blog/jellyfin-comics-manga-kavita-komga-self-hosted-reader-2026).
- Frequency: medium.
- Bookx: PARTIAL. OPDS and a web page serve phone apps; gestures and large touch controls exist (roadmap R13,
  X5 adaptive layout noted); no native mobile app or offline web client.
- Proposal: a PWA for the content server with service-worker offline cache and a downloaded-books page (roadmap
  P10); keep a GTK adaptive layout for Phosh/Linux phones. L, P3.

### PP-G-28 Cover quality

Embedded covers are tiny; sources return low-resolution or wrong images; covers are not written into files.

- Evidence: [CWA-NextGen #304: Kindle high-res covers plugin](https://github.com/new-usemame/Calibre-Web-NextGen/issues/304),
  [CWA README: chosen covers missing on other devices](https://github.com/crocodilestick/calibre-web-automated),
  [MobileRead: cover fetch plugins unreliable](https://www.mobileread.com/forums/showthread.php?t=347944).
- Frequency: medium.
- Bookx: PARTIAL. Cover generation and crop (`crates/bookx-sources/src/cover.rs`, roadmap L13) and cover fetch in
  the edit dialog; no resolution/aspect scoring or candidate picker verified.
- Proposal: show up to N candidate covers with size and aspect, pick the highest resolution by default, write
  the cover into the file and the thumbnail cache, "extract page as cover" for comics (R54). S, P2.

### PP-G-29 Tagging and organisation

Tag soup, no hierarchy, no smart shelves, and per-user shelves are inconsistent.

- Evidence: [CWA 4.0: Magic Shelves added](https://www.reddit.com/r/selfhosted/comments/1qqb2pd/calibreweb_automated_v400_released_smart),
  [Kavita: smart filters, collections, reading lists](https://www.kavitareader.com),
  [MobileRead author/tag fields in one column](https://www.mobileread.com/forums/showthread.php?t=125883).
- Frequency: medium.
- Bookx: PARTIAL. Collections, reading status, a query language (`bookx-core::query`) and SQLite collections
  exist (roadmap L5, L22). Tag browser and saved searches in the UI are partial per Part 3 (L18, L4).
- Proposal: "Smart shelf" = saved query shown in the sidebar and in OPDS; hierarchical tags with counts; drag to
  assign. S, P2.

### PP-G-30 Search inside books

Full-text search over the whole library is slow or absent in most managers.

- Evidence: [Calibre bug: search extremely slow](https://bugs.launchpad.net/bugs/1862972),
  [CWA 4.0: search performance overhaul](https://www.reddit.com/r/selfhosted/comments/1qqb2pd/calibreweb_automated_v400_released_smart).
- Frequency: low (few complaints about full-text specifically; these two are about general search).
- Bookx: DONE. Background FTS5 indexer, `text:` term that combines with other fields (`src/fulltext.rs`, roadmap
  "Full-text search of book contents"); in-book search exists in the reader.
- Proposal: keep; add a 100k-book budget (PP-G-12). S, P3.

### PP-G-31 Long-term preservation

File formats and store behaviour change; books can be updated or removed remotely; folder layouts become
unreadable without the original tool.

- **Update 2026-11-12 (metadata track, `docs/metadata-notes.md`):** the preflight check runs the validator before sending or converting; validate-on-import and fixity checksums are still open.
- Evidence: [AkitaOnRails: remote deletion, updated covers, format changes](https://www.akitaonrails.com/en/2026/07/30/removing-drm-from-kindle-ebooks-in-2026),
  [Calibre FAQ: books marooned in its folder structure](https://manual.calibre-ebook.com/faq.html),
  [Kindle formats, KFX vs AZW3](https://www.rsinc.com/how-to-remove-drm-from-kindle-books-in-2025.php).
- Frequency: medium.
- Bookx: PARTIAL. EPUB-first editing and a validator exist; no fixity checksums, no periodic verification, no
  keep-original-on-convert policy surfaced in the UI.
- Proposal: store a SHA-256 per file at import, "Verify library" that detects bit rot or external edits,
  keep the original next to converted copies by default, validate on import and flag problems. M, P2.

### PP-G-32 Ingest folders are fragile

Drop-a-file ingest breaks when files are partly written, owned by the wrong user, or downloaded in place.

- Evidence: [CWA README: copying into the ingest folder can cause duplicates and corruption](https://github.com/crocodilestick/calibre-web-automated),
  [BookLore feature list: BookDrop imports](https://github.com/booklore-app/booklore),
  [CWA #388: permission errors after migration](https://github.com/crocodilestick/Calibre-Web-Automated/issues/388).
- Frequency: medium.
- Bookx: NOT YET as an app feature (roadmap L12 "folder watcher" is a Part 1 row; Part 4 shows only the
  catalogue watcher, not a watched import folder).
- Proposal: a watched folder (`notify` crate) that waits until a file is stable (size/mtime quiet period, ZIP
  end-of-central-directory present), imports into a "Review" shelf, and never deletes the source unless asked.
  S, P1.

### PP-G-33 Destructive operations without undo

Libraries get reorganised, files renamed, permissions changed, or categories mis-assigned after an update, with
no way back.

- Evidence: [Notebookcheck: update classified books as "physical", permissions changed on uninstall](https://www.notebookcheck.net/Booklore-The-Plex-of-self-hosted-e-book-libraries-sparks-licensing-controversy-and-community-backlash.1287933.0.html),
  [note.com: do not tidy Calibre's library folders by hand](https://note.com/yaminabei/n/n07bcf157f4aa?hl=en),
  [r/Calibre: wrong metadata overwritten](https://www.reddit.com/r/Calibre/comments/1hp6bs1/calibre_keeps_changing_the_metadata_of_books_to).
- Frequency: medium.
- Bookx: NOT YET at library level (roadmap L37 library-level undo is only an idea). The editor has automatic
  checkpoints before each tool (`src/editor_tools.rs`) and Send to device never overwrites (`src/devices.rs`).
- Proposal: an operation log in SQLite (metadata edits, merges, deletes) with Undo in a toast and a History page;
  deletes go to a trash folder for N days. M, P2.

---

## Top 10 things Bookx should do next

1. **Reader as a KOSync client with precise matching** (PP-G-8). Biggest cross-product pain; the server half exists.
2. **Annotation portability** (PP-G-10 and PP-G-9): wire the existing Readwise/clippings exporters, add Obsidian/Anki, add importers for My Clippings.txt and Kobo's database, fix `docs/bugs.md` #2.
3. **Metadata that you can trust** (PP-G-1 and PP-G-2): provenance, source health, a review queue, and a file-vs-catalogue drift check.
4. **Watched import folder with a review queue** (PP-G-32), combined with auto-fetch at import.
5. **Scheduled backups, restore UI and a library "Check"** (PP-G-14), then an operation log with undo (PP-G-33).
6. **Series and author tidy-up plus a series view** (PP-G-3), and number-word handling and an import-time check in the duplicate finder (PP-G-4).
7. **Send-to-device preflight** (PP-G-6): Amazon-safe validation and a real-device test matrix for Kindle and Kobo.
8. **Accessibility** (PP-G-19): AT-SPI text for the reading canvas, keyboard-only reading, TTS; it is a gap in a native GTK app that web readers cannot match.
9. **Vertical, RTL and CJK in the canvas** (PP-G-21), since the roadmap already commits to it (R4, R56).
10. **100k-book budgets and a single-writer lock** (PP-G-12 and PP-G-13), so large and shared libraries stay safe.

Next after these: Goodreads/StoryGraph CSV import and export (PP-G-16), MTP (PP-G-11), comic page reader (PP-G-24), DRM detection messages (PP-G-7).

## Differentiators Bookx could own

- **One app that speaks every sync protocol**: KOSync server and client, OPDS, Kobo/Kindle send, so a Bookx library replaces Calibre plus Calibre-Web plus a kosync container.
- **Honest sidecar and file-first data**: catalogue and files stay in step, OPF export, documented schema; answers Calibre lock-in and the Booklore/Readarr abandonment stories.
- **Built-in editor and validator as a send-to-device preflight**: neither Kavita, Komga nor Calibre-Web has this, and CWA needs a separate EPUB fixer.
- **Commonplace book**: cross-book notes with real export and import, stable ids, local vocabulary and flashcards.
- **Typography on the desktop**: Pango, hyphenation, and (when done) Knuth-Plass, vertical text and RTL in a native toolkit.
- **No telemetry, auditable network use**: a published "what Bookx connects to" list.
- **Accessible by design**: a native GTK reader with AT-SPI text would beat Electron/web readers on Linux.
- **CLI/headless parity** (D7) for scripted libraries, which the *arr family and Calibre-Web users keep reimplementing with shell glue.

## Unverified claims and gaps in this research

- **Frequency labels** are my judgement from the threads and trackers I opened, not counts; many results were search snippets, not full threads.
- **Calibre 9.18 KOReader sync**: taken from a YouTube summary of the release, not from the Calibre changelog. Check [calibre-ebook.com/whats-new](https://calibre-ebook.com/whats-new) before relying on it (PP-G-8).
- **Kobo's rebuilt sync** (MakeUseOf, citing the Kobo CEO): rollout status and effect on sideloaded books not verified.
- **Booklore details** (shutdown, Grimmory fork, "physical" categorisation, permission change): from XDA and Notebookcheck; I did not check the repositories or the community's own account.
- **Telemetry** (PP-G-18): only one forum source mentions analytics in a Servarr app; treat "privacy" as a weaker claim than the others.
- **CJK/RTL** (PP-G-21): only two trackers found; language-specific forums (Japanese, Arabic) were not searched.
- **Accessibility** (PP-G-19): evidence is about Thorium and Calibre; I did not find user complaints about GTK-based readers specifically, and I did not test whether `canvas.rs` exposes any AT-SPI text.
- **Bookx status** is based on code and roadmap reading plus greps. I did not run the app or the tests. Items where I could not confirm absence: write-back on every edit path (PP-G-2), "export back" to Calibre (PP-G-15), a comic page reader in the UI (PP-G-24), number-word duplicate matching (PP-G-4), whether the reader touches the KOSync store (PP-G-8).
- **Legal**: DRM removal is regulated differently by country; the doc only proposes detection and clear messages, and the owner should decide whether Bookx does anything more (including LCP).
- **Not researched**: Moon+ Reader, Koodo and Foliate bug trackers beyond the items linked, LazyLibrarian forums, Hardcover/StoryGraph app reviews on reading-statistics accuracy, and Apple Books / Google Play Books.
