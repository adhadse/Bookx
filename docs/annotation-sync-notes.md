# Annotation and progress portability: notes and caveats

Internal. Work of 2026-11-12 on PP-G-8, PP-G-9, PP-G-10 (`docs/pain-points-ebook-managers.md`). User-facing text is in
`wiki/docs/annotations.md` and `wiki/docs/progress-sync.md`.

## What was built

| Piece | Where | Tests |
|---|---|---|
| One sync planner: `device_sync::plan` maps `bookx_serve::syncplan` (pairing, progress rule, highlight/note rules, conflicts, "restarted" gate) onto library types; `devices::{library_sides,device_sides,full_sync_plan}` removed | `src/device_sync.rs`, `crates/bookx-serve/src/syncplan.rs` (`pair`, `same_highlight` now public) | `device_sync` unit tests (conflicts, restart gate, authors), `devices` test of a real-schema Kobo, scenarios `device_sync_ui`, `device_auto_sync`, `device_window`, … |
| Importers: Kindle clippings, KOReader JSON / Markdown, Bookx JSON, calibre JSON (lenient walk) | `crates/bookx-lookup/src/import.rs` | unit tests incl. round trip with our own Kindle and JSON exports |
| Importers needing other crates: KOReader `.lua`, Kobo DB (temp copy), folder scan, matching, plan, apply | `src/annotation_import.rs`, `kobo::read_book_notes` | unit tests; scenarios `annotations_import`, `annotations_import_devices` |
| Exporters in the UI: Markdown, HTML, JSON, text, Obsidian, Kindle clippings, Readwise body; one book and whole library | `src/annotation_export.rs`, `src/ui/annotations.rs`, menu entries | unit tests; scenario `annotations_export` |
| KOSync client (protocol, partial MD5, file-name id) | `crates/bookx-serve/src/kosync_client.rs` | in-process against `Server` with `KoSync`; skipped with `NET=0` |
| Client rules (decision, mapping, hold-back) and settings | `src/kosync_sync.rs`, `src/ui/kosync.rs`, reader hooks (`open_book`, `position`, `close_book`) | unit tests; scenarios `kosync_client`, `kosync_client_stay` (second client: `build-aux/kosync-peer.py`) |

## Decisions

- **Merge of planners.** `device_sync` keeps its public types (`Plan`, `BookPlan`, `Summary`, `apply_book`, `Locator`) because
  the Device window, the banner and the scenarios depend on them; only the decisions moved to `syncplan`. Highlights are
  mapped back to the library via an index carried in `Annot::when`. A device note that carries one of our highlight ids
  is rewritten to the library text first, so the id identity of the old planner survives.
- **Behaviour changes from the merge** (all intended by `syncplan`'s rules): same-title books with different authors are
  not paired; two library books with the same title and no author tie-break are reported unmatched instead of picking the
  first; a device ahead of a library copy that was read more than a day later is *not* pulled forward ("restarted" gate,
  reported in the summary); a library book at ≥ 100 % counts as finished (before: any progress < the device's was moved).
  Highlights match when one text is inside the other (≥ 12 chars), so a Kindle highlight that was extended counts as the
  same quote.
- **Conflicts** (same highlight, different notes) are listed in the sync summary and change nothing on either side. The
  device's note is not copied into the library (no merge of two different thoughts); it stays on the device.
- **Import is additions only.** Unplaceable quotes (not found in the book's text, even with the loose match for quotes,
  dashes, spaces and soft hyphens) are *not* imported at a made-up position; they are counted in the review. Bookmarks and
  notes with no quote are skipped (a Bookx bookmark needs a place in the book).
- **KOSync.** `x-auth-key` is the MD5 of the password, as KOReader sends it; the server stores whatever key it is given.
  Document ids: partial MD5 over 1 KiB at offsets 0 and `1024 << 2i` (i = 0..10), stopping at the first empty read; file
  name MD5 of the name without folder. Implemented from the protocol description and memory of KOReader's
  `util.partialMD5`; checked only against an independent Python implementation written by the same author.
- **Secrets.** The KOSync password was already moved to the keyring by `Config::secret_fields` (the note that said it
  lives in the config file was out of date). Wave 4 added `readwise-token` and `tts-api-key` there; `BOOKX_TTS_API_KEY`
  is copied into the setting once (`tts::migrate_env_key`) and stays as a fallback.

## Caveats and not verified

- **Not tested against real KOReader, sync.koreader.rocks, a real Kindle or a real exported calibre file.** The calibre
  parser is a lenient walk because I could not check the real layout of `calibredb annotations export`. The Kindle parser
  knows the English clippings format and the type words of a few languages; other date formats are dropped (the entry is
  kept without a date).
- **Percentage mapping** uses Bookx's chapter weights (words per chapter). KOReader's percentage is of its own page/character
  layout, so landing spots differ by a page or two, more with images. Bookx ignores the incoming xpointer and writes the
  chapter start (`/body/DocFragment[N]/body`) as its own, so KOReader opens at the chapter start. A PDF (page-number
  `progress`) from KOReader would be mapped by percentage too.
- **Offline queue (wave 4).** A push that fails with a transient error is kept in `kosync-queue.json` (one entry per book),
  retried at start, every 60 s and on `NetworkMonitor::network-changed`. A push in flight when the whole app quits is
  queued only if its failure was seen; one the OS killed mid-request is lost.
- The hold-back rule asks the server before pushing until the question is answered; if the server cannot be reached the
  push fails anyway. A "Stay Here" answer means later pushes overwrite the other device's position.
- Matching books for import uses `syncplan::pair` (title, authors): a title with a subtitle or series prefix that the
  library does not have in the same form is "not in your library". Fuzzy matching and ISBN are not used.
- The sync summary and review are plain text lines in a dialog; there is no per-highlight checklist. Screenshots or
  mockups for the wiki pages were not made (the pages describe the dialogs in text).
- Not done: writing highlights into `KoboReader.sqlite` (we never write that database for annotations), reading Kindle's
  own annotation files (`.mbp`/`.azw3r`: undocumented binary, nothing to verify a parser against, and `My Clippings.txt`
  has the same highlights), Calibre viewer annotations from `metadata.db`.

## Wave 4: annot-extras (2026-11-13)

| Piece | Where | Tests |
|---|---|---|
| Notes & Highlights window (search, filters, grouping, edit, delete, export, jump to the passage; PDF rows show the page) | `crates/bookx-lookup/src/notes_query.rs` (pure query), `src/notes_view.rs` (rows from books, edit/delete, export items, Anki bytes), `src/ui/notes.rs` (dialog, dev hook `library:notes:CMD`), `Act::Notes` + menu entry in `ui/library.rs`, jump via `ui::notes::{set_jump,take_jump}` read in `app.rs` | lookup unit tests, `notes_view` tests, scenarios `notes_window`, `notes_edit_export`, `notes_open_passage` |
| Stable ids | `bookx_lookup::ids::content_id` (sha-256 of normalised title, author, quote; `bx-` + 24 hex), `id` in the JSON export, `annotation_ids::{migrate_library,incoming_id}` (startup pass fills empty / clashing library ids; imports get the content id) | `ids`, `annotation_ids`, JSON export test |
| Anki | `bookx_lookup::anki` (basic / cloze notes from highlights and vocabulary, TSV/CSV with `#guid column` etc., `.apkg` writer: schema 11 `collection.anki2` through `rusqlite` + zip) | unit tests incl. re-opening the package; the UI scenario opens the package with Python's `sqlite3` |
| Readwise upload | `src/readwise_sync.rs` (batches of 100, ledger `readwise-sent.json`, fingerprint = quote + note + time, `Poster` trait, `Http` over `bookx_serve::httpc`), token in `Config.readwise_token` (keyring) | tests against a local fake server, scenario `notes_readwise` against `build-aux/fake-readwise.py` |
| KOSync offline queue | `src/kosync_queue.rs`, `ui/kosync.rs` (`flush_queue`, `start_queue`) | unit tests, scenario `kosync_client_queue` |
| Kindle | `My Clippings.txt` on a mounted Kindle was already read by `read_folder`; test added that nothing on the Kindle is touched and binary `.sdr` files are ignored | `a_mounted_kindle_gives_its_clippings_and_nothing_is_written` |

Decisions and caveats of wave 4:

- **Stable id is derived, not stored.** The library keeps its UUIDs (devices and the sync refer to them); the content id
  is computed for exports and Anki guids. Two highlights of the same quote in one book therefore share a content id
  (the export lists both, Anki gets one card).
- **Anki `.apkg` is not verified against a real Anki.** The schema, model and deck JSON follow Anki's documented layout
  (legacy schema 11, which every Anki version imports); the test only proves it is a valid SQLite collection with the
  expected rows. The checksum field is a SHA-1 written here (verified with the standard test vectors). A cloze card hides
  one word chosen by a rule (longest word of 5+ letters), which is a rough heuristic.
- **Readwise not tried against readwise.io.** The body is the documented `POST /api/v2/highlights/` shape; 401 / 403 / 429
  / 5xx are handled by status only (the HTTP client does not expose `Retry-After`). A highlight whose note changes is
  sent again; Readwise's own de-duplication decides what that does on its side.
- **The Notes window lists at most 200 rows at a time** (plain `ListBox` rows, "Show more"); a library with tens of
  thousands of highlights has not been measured. Chapter titles are only looked up for exports (opens each book once);
  the list shows "Chapter N".
- A race in the dev scenarios: `GtkSearchEntry` emits `search-changed` after 150 ms, so the scenarios wait after a
  `search=`. Real use is unaffected.
- The Notes window changes highlights in the shared library; a reader open on the same book keeps its own copy of the
  highlight list until it reloads.

## Wave 10: bookmarks from devices, and a type filter in Notes & Highlights

Owner question: "where in the library can I see the bookmarks, here or from the synced device?" Bookmarks lived only in the
reader's sidebar, synced bookmarks were counted and thrown away.

- **Model.** No change to `core::library::Bookmark` (id, spine, frac, label, created): 12 constructors in 5 crates and the
  catalogue format stay as they are. What a bookmark needs more is kept in a ledger: `bookmark_sync::Entry` (book, bookmark id,
  source `KOReader (device)` / `Kobo`, `approx`, the device's hint, `made_here`). Wireless: `Ledger::bookmarks`
  (`annotation-sync.json`); cable: `bookmark_sync::Store` (`<library>/device-bookmarks.json`, same entry type). The words the
  reader wrote on a device bookmark are its `label`; without them it is `Page N` (the device's page) or the chapter.
- **Place** (`bookmark_sync::resolve_place`, a `Geometry` trait so it is unit tested without a book): xpointer ->
  `DocFragment[N]` + `fraction_in_markup` (exact); Kobo dog-ear: chapter file + `ChapterProgress` (exact); chapter title (start of
  the chapter, refined by the percentage when it falls inside); percentage of the book; device page / (text length / 2000)
  as a share of the book. Everything but the first two is `approx` and shown as "approximate place". Nothing found: the start
  of the book, flagged. The device's page numbers are in *its* layout, so the page guess is a guess.
- **Duplicates.** Only exact places are compared with the book's bookmarks: same chapter and closer than 1 % of it, or closer
  than 4 % and made the same day -> linked, nothing added, the bookmark of the reader untouched. A rough place never merges
  (two pages named only by their chapter are two bookmarks). A Kobo dog-ear and a KOReader bookmark of the same page of one
  book are one bookmark.
- **Nothing is ever deleted.** The ledger entry stays when the reader deletes the bookmark (tombstone: `import` returns
  `Known`) and when the device deletes it. An older sync only remembered bookmarks (`State::Bookmark` with no entry): they are
  imported at the next merge.
- **Notes & Highlights.** `NoteRow.kind` (`RowKind`) and `Filter.kind` (`KindFilter`: all, highlights, notes = highlights with
  a note, bookmarks). The choice is `Config::notes_kind`. `notes_view::rows_of` lists bookmarks after the highlights of each
  book, and the sidebar count is `rows().len()` (still identical to the list with *All*). Deleting a bookmark shows the Undo
  toast (the highlight delete keeps its confirmation); the bulk delete handles both. Open: `notes::set_jump(book, bookmark id)`
  -> `Input::GoHighlight(id)` finds a bookmark when no highlight has that id -> `load_chapter(spine, Some(frac))`.
- **Dev hooks.** `library:notes:kind=`, `library:notes:kinds`, `reader:bm-where`, `reader:side-state` prints `side_bookmark:`
  lines, `koannot:dump` prints `koannot_bm:` lines; scenarios `koreader_annotations_bookmarks*`.
- **Not done / caveats.** The reader does not flash the landing line of a bookmark (it opens the page that holds the place).
  Nothing was tried against a real KOReader or Kobo: the KOReader record shape (`page` xpointer, `pageno`, `datetime`) is from
  the owner's sidecar (`a_page_bookmark_is_no_highlight`) and from the plugin's JSON; the Kobo dog-ear columns (`Type='dogear'`,
  `ContentID` = `<volume>!<chapter path>`, `ChapterProgress`) are from the real schema in `kobo.rs` but the exact `ContentID`
  form of a real device's dog-ear is unverified, and a `kepub` split of a chapter into several files would place it by file only.
  The bookmark of a PDF is a page (`spine + 1`), bookmarks of PDFs are not synced from devices. Bookx -> device: Bookx's own
  bookmarks are still not sent to KOReader.
