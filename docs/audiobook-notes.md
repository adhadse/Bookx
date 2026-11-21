# Audiobooks (PP-G-25): notes

Internal notes: design, decisions, test inventory, caveats. The user-facing text is `wiki/docs/audiobooks.md`.

## Status (2026-11-20, wave 6)

DONE: import (folder, zip, single file), tags/chapters/cover reading, catalogue entries, library badge/filter/details,
player page, speed with pitch kept, sleep timer, bookmarks with notes, resume, MPRIS, listening statistics, tests.
NOT DONE (on purpose or for lack of time): see "Not done" below.

## What was built

| Piece | Where |
|---|---|
| Format detection, ID3v2/MP4/Vorbis/Ogg/FLAC/WAV readers, chapters, covers | `crates/bookx-audio` (`format`, `id3`, `mp4`, `vorbis`, `ogg`, `probe`, `text`) |
| Folder/zip/file scan, natural order, safe zip unpacking | `bookx-audio::scan` |
| Parts, chapters and the position arithmetic (chapter math, resume, time left) | `bookx-audio::model` |
| Per-book state (position, speed, bookmarks) | `bookx-audio::state` |
| Decoding (Symphonia 0.5, pure Rust) and seeking | `bookx-audio::decode` |
| Speed change with the pitch kept (WSOLA) | `bookx-audio::stretch` |
| Playback engine (worker thread, sinks: command-line player, null, memory) | `bookx-audio::player` |
| Generated test audio (WAV, FLAC, Ogg skeleton, MP4 atoms, ID3) | `bookx-audio::testgen` |
| Import into the catalogue, `Listening` session (sleep timer, stats, saved place, catalogue progress) | `src/audiobook.rs` |
| The player page, details group, sidebar row, cover badge, Add Audiobook, stats group | `src/ui/audiobook_player.rs` |
| App wiring (`Msg::PlayAudio`, `audio:` dev hooks), library wiring (badge, sidebar, details, drop, import hook) | `src/app.rs`, `src/ui/library.rs` (small additive edits), `src/import.rs`, `src/book_formats.rs` |

## Decisions

* **An audiobook is a catalogue book** whose file is a small generated EPUB (title, author, narrator, chapter list). Everything
  that expects an EPUB (details, covers, shelves, export, search, full text) keeps working; the audio is an `audiobook`
  resource (`Resource.path` = the folder or file, absolute) plus one `FormatFile` per book with the audio format, so
  `format:m4b` and the *Formats* sidebar group work without any change to the query code. `audiobook:yes` (existing) finds
  them. The marker of "made by an import" is the identifier scheme `bookx-audiobook` (so an e-book with a linked audiobook is
  not mistaken for one). Opening such a book (`Msg::Read`) opens the player instead of the reader.
* **The audio stays where it is** (linked, like the Linked layout). Copying a library of audiobooks (tens of GB) into the library
  folder was not wanted by default, the import runs on the worker thread without a destination, and Calibre users know the
  linked model. Zips are unpacked under `~/.config/bookx/audiobooks/unpacked/` (the one place Bookx owns). A "copy into the
  library" option is a possible follow-up (needs a destination in `Prepared`).
* **Position storage.** One JSON file per book in `~/.config/bookx/audiobooks/<sha256-of-path>.state.json`: position (ms),
  speed, bookmarks with notes. For audiobooks made by an import the position is mirrored into the catalogue
  (`Book.position` = chapter + fraction, `progress`, `last_read`), so the library shows the percentage, *Continue reading*
  and the Read status; for an e-book with a linked audiobook only the state file is written, so the reading place of the
  e-book is never overwritten.
* **Scan cache.** `~/.config/bookx/audiobooks/<sha256-of-path>.json` (the `AudioBook` model): a book of 500 chapters opens
  without scanning again; a missing part invalidates it.
* **Chapters** come from the file (MP4 `chpl`, QuickTime chapter text track, ID3 `CHAP`, `CHAPTERnnn` Vorbis comments); without
  marks one chapter per part.
* **Sound output** uses the same mechanism as read aloud (a command-line player fed through a pipe: `pacat`, `pw-cat`, `aplay`),
  not the same code: `bookx_tts::sink::CommandSink` is mono and sentence-based, the audiobook sink is stereo and writes a
  continuous stream. `BOOKX_AUDIO_PLAYER` is the override (like `BOOKX_TTS_PLAYER`), `BOOKX_AUDIO_SINK=null[:N]` plays
  nothing and takes `1/N` of the time (UI scenarios). A pause flushes the pipe (kills the player and starts a new one on
  resume) so it is exact; the position shown is what was written minus the pipe's latency.
* **Sleep timer** reuses `bookx_tts::SleepTimer` (`Off`, `At`, `EndOfChapter`). End of chapter is exact: the engine cuts the
  chunk at the chapter end (`Player::pause_at`).
* **Statistics.** Listening sessions are recorded with `bookx_lookup::stats::StatsStore` in `listening.json` next to the
  reading statistics (same session rules: 5 s minimum, merge within 60 s, streak/day logic from `reading_stats::summarize`).
  Kept apart from reading so reading speed (words per minute) and pages are not skewed; the statistics window has a
  *Listening* group.
* **Speed** 0.5x to 3x with pitch preserved by WSOLA (40 ms windows, 10 ms search, correlation on every 4th sample). Speech
  sounds natural; music would show artefacts. A minute of 44.1 kHz stereo is stretched in a fraction of a second (budget in
  `crates/bookx-audio/tests/perf_budgets.rs`).

## Security (docs/security.md §8)

* Untrusted bytes: ID3v2, MP4 atoms, Vorbis comments, Ogg pages, FLAC blocks, WAV chunks, zip entries, the state and manifest
  JSON. Budgets in `bookx_audio::limits` (tag 24 MiB, `moov` 48 MiB, 5,000 chapters per file, 20,000 items per tag, box depth,
  Ogg pages, 50,000 zip entries, 16 GiB unpacked, depth 6 for folders, 64 images). Bad-input tests: every prefix and many
  byte flips of generated and real files for each parser (`hostile_*`, `truncated_and_flipped_*`), absurd sizes (a frame that
  claims 4 GiB, a box of 2^64-1 bytes, a `moov` over the limit, a 5,000 deep box chain, a bookmark file of 1,500 notes).
  There is no cargo-fuzz target yet (same state as the other parsers).
* Zip entries: names checked by `scan::safe_rel` (no absolute, `..`, `.`, backslash, drive letters, NUL, hidden, `__MACOSX`, over
  300 bytes), symlink entries skipped, only audio and picture entries unpacked, the sizes in the zip are not believed (the copy
  is capped), entry count capped.
* Folders: symlinks are not followed (a link could lead out of the folder); hidden entries skipped.
* No external references: tags are text only; no URL in a tag is ever fetched. Covers are accepted only when the bytes start like
  JPEG, PNG, GIF or WebP.
* No process other than the sound player (`pacat`/`pw-cat`/`aplay` found on `PATH` by name, as read aloud does; arguments are
  rates and channel counts that we compute; the override `BOOKX_AUDIO_PLAYER` is an absolute program path with arguments from the environment (no shell since 2026-11-21), same as
  `BOOKX_TTS_PLAYER`, and is not reachable from a file).
* New dependency: `symphonia` 0.5.5 and its codec/format crates (MPL-2.0, pure Rust, `#![forbid(unsafe_code)]` is not set by
  them; they contain no `unsafe` in the decoders we use except SIMD behind a feature that is off). Listed in `NOTICE.md`.
  RUSTSEC check: no advisory for `symphonia` 0.5.x at the time of writing (run `cargo deny` in CI).

## Tests

* `crates/bookx-audio`: 83 unit tests (tags, hostile input, chapters, folder/zip scans, zip name safety, symlinks,
  decoding of generated and real files, seeking, stretching, player engine with the memory sink: order of parts, speed, pause,
  seek, `pause_at`, errors, Opus refused) and 2 performance budgets (`tests/perf_budgets.rs`: a folder of 500 chapters, a
  minute stretched).
* `src/audiobook.rs`: 13 unit tests (catalogue import, own vs linked, zip, cache, resume, speed and bookmarks surviving a
  restart, sleep timer, end of chapter, statistics, chapter buttons, progress texts).
* `build-aux/ui-tests-audiobook.sh`: 12 Broadway scenarios (`audio_*`): import of a folder, an M4B and a zip, filters,
  player controls, speed clamps, sleep timers, bookmarks, resume, statistics, finishing, unplayable Opus, a11y/icons, 900x600.
* `build-aux/session/scenarios_audiobook.py`: 3 real-session scenarios (real double click, space, arrows, page keys, menus,
  Escape; MPRIS over the session bus; dark 900x600) with screenshots.

## Caveats and not done

* **Nothing was heard on a real sound card.** The tests use the null sink, the memory sink and decode checks; the command-line
  player path (`pacat`/`pw-cat`) is a pipe whose arguments are unit tested, and its start/stop was not listened to. Real
  hardware check needed: play a book through PipeWire, pause/resume (the player is killed and restarted), 3x speed, the end of
  a part (gapless between parts is by continuous writing; there may be a click if the sink restarts at a rate change).
* **Opus** (`.opus`) is imported and listed but not played (Symphonia 0.5/0.6 have no Opus decoder). A pure-Rust Opus decoder
  would be a separate dependency decision.
* **Media keys:** MPRIS Play/Pause/Next/Previous work (tested over D-Bus); seeking and the position property are not exposed
  (`CanSeek` false). The real-session sandbox has no media-keys daemon, so the physical key could not be tested there.
* **Sync of the position** to other devices: nothing generic exists. KOSync is for e-book positions (xpath/percentage of an
  EPUB) and is not extended; the place is only local (catalogue + state file). Documented in the wiki.
* **One place for e-book and audiobook** (wave 11): `bookx_audio::link` maps positions (chapters with the same title are anchors, linear interpolation in between,
  share of text by spine item size); *Continue in Audiobook* / *Continue Reading* rows in the details panel (`src/audiobook.rs`: `continue_in_audiobook`, `continue_reading`).
  Manual, not automatic; weights are file sizes (images, markup count), so the match is rough without titles.
* **Opus: still not played.** No Opus decoder is in `Cargo.lock` (Symphonia 0.5 has none); adding one (`libopus` binding = C code and unsafe, or an unvetted pure-Rust crate) is a dependency decision left to the owner.
* **Position sync** to other devices: not built; the `/bookx` protocol carries e-book positions only. The local place (state file + catalogue) is the single source.
* No fade-out for the sleep timer, no per-chapter speed, no "trim silence", no volume boost, no equaliser.
* The details panel's *Play in Bookx* loads the cached scan on the UI thread; a first load of a linked folder of hundreds of
  files without a cache scans on the UI thread (about 0.1 s for 500 WAV parts, more for MP3 files without a Xing header).
* `bookx-cli library add` does not import audio (the importer is in the app: `Add Audiobook`, drag and drop, `audio:add=`).
* A library moved to another machine keeps links to audio paths of the old one (the same as Linked layout books).
* MP4 chapter text tracks with 32-bit offsets and `co64` are read; QuickTime chapters of the `tx3g` kind with styled text are
  read as plain text (the style atoms are ignored).
* AAC priming (encoder delay) is not trimmed unless the file declares it (gapless metadata): up to ~50 ms of silence at the
  start of a part.
