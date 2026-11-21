# KOReader highlight and note sync over Wi-Fi

Internal design and status notes (2026-11-20). User text: `wiki/docs/koreader-sync.md`. Owner decision D10 in
`docs/bookx-feature-roadmap.md` Part 2: **wireless sync is supported only for KOReader** (any device that runs it).
Stock Kobo/Kindle/other firmware: USB only.

## Why a plugin

KOReader's *Progress sync* (KOSync) moves one position per book. It has no standard protocol for highlights, notes or
bookmarks, and the other routes (cloud storage of the `.sdr` folder, Readwise export) are not Bookx-shaped. So:

* **(a) built, the main path**: a small KOReader plugin written for Bookx (`koreader-plugin/bookx.koplugin/`, MIT,
  own code, nothing copied from KOReader or other plugins) that talks to Bookx's built-in server on the same connection as the KOSync reading-progress routes. The umbrella endpoint is **`/bookx`** (the backend is more than KOSync); there is **no** `/kosync` mount and no root-level mount any more (owner decision, 2026-11-20): `/kosync/...` and `/users/auth` answer 404 (401 when the library has a Basic-auth user). The plugin suggests `/bookx`; a bare host gets `/bookx` appended. Anyone who typed a `/kosync` address into KOReader or the plugin must change it to `/bookx`.
* **(b) already there, the fallback**: with the device mounted by USB, Bookx reads the `.sdr` sidecars
  (`device_sync`, `annotation_import`). Unchanged.

## Protocol (`crates/bookx-serve/src/annotation_sync.rs`)

Same account and headers as KOSync (`x-auth-user`, `x-auth-key` = MD5 of the password); the same document id (KOReader's
partial MD5 of the book file, 32 hex digits). Routes below `/bookx` (the KOSync routes `users/create`, `users/auth`, `syncs/progress[/<doc>]`, `healthcheck` live there too):

| Route | Meaning |
|---|---|
| `PUT /bookx/annotations/<md5>[?since=N]` | body `{origin, device, device_id, title, authors, annotations:[...]}`; idempotent upsert; the answer is `{document, seq, result:{accepted,unchanged,stale}, annotations:[records with seq > N]}` |
| `GET /bookx/annotations/<md5>[?since=N]` | the records (tombstones included) |
| `GET /bookx/info` | no login: `{name:"bookx", protocol, min_plugin_protocol, progress, annotations}` (added 2026-11-21). The plugin's *Test connection* reads it: server `protocol` below the plugin's → "Bookx too old"; `min_plugin_protocol` above the plugin's → "update the plugin"; `annotations:false` → "switched off"; 404 (an older Bookx) → carry on. Raise `annotation_sync::PROTOCOL` / `MIN_PLUGIN_PROTOCOL` only for a change that breaks an installed plugin |

Annotation: `id` (unique in the book), `kind` (`highlight`, `bookmark`), `text`, `note`, `color`, `drawer`, `chapter`, `pos0`,
`pos1`, `page`, `percentage`, `created`, `updated` (seconds), `deleted`. The server adds `origin`
(`koreader` / `bookx`), `device`, `seq` (global change counter).

Rules:

* **Idempotent upsert** keyed by `(user, document, id)`: the same request twice changes nothing (and does not move `seq`).
* **Conflict**: last write wins per annotation by `updated`. A tie: a live annotation beats a tombstone, otherwise the stored
  version stays. So a clock tie never loses an annotation. An `updated` more than a day in the future is refused (a device with a
  wrong clock must not win every later conflict).
* **Deletion** = tombstone (`deleted: true`), kept 365 days so every device hears of it; an older edit cannot revive it.
* **Validation**: whole request or nothing (422), strict types, no control characters in ids/names, ids limited to
  `[alnum - _ . : @ +]`, the document id must be 32 hex digits (so no path of any kind reaches the file system; the store is a
  single JSON file and keys are never used as paths).
* **Limits**: body 1 MiB (transport and handler), 1000 annotations per request, 20 000 characters per text/note, 5000
  annotations per book, 5000 books per user, 120 requests per user per minute (429 + `Retry-After`).
* **Auth**: KOSync's credentials, constant-time compare (reuses `KoSyncStore::check_user`); the annotation routes are off unless
  the Share window's *Sync highlights and notes* switch is on (404 otherwise). No registration here: accounts are made through
  KOSync's `/users/create`.
* Storage: `koreader-annotations.json` in the data folder, atomic write (temp file + rename), rolled back in memory if the write fails.

## Bookx side (`src/annotation_sync.rs`, `src/annotation_sync_ui.rs`)

A timer (4 s, only when the server's `seq` moved) or *Sync now* merges the store into the library under these rules:

* Which book? By the file's partial MD5 (cached per file state in the ledger); else by title + authors through
  `syncplan::pair` when exactly one book fits and the authors do not contradict. Else the document is counted as "not in your
  library" and retried next time.
* **Anchoring: the text decides, xpointers are hints only** (and are not used). The quote is found through the same
  `Locator` as the device sync and imports (`BookLocator::locate_in`, text-verified, `core::anchor::resolve`, chapter
  name as a hint). A quote that is not found becomes an **unanchored highlight** (`anchor::unplaced`: kept with its text and
  note, listed in Notes & Highlights as *Could not be located in the book* with *Locate again*, painted nowhere).
* A quote the library already has (`syncplan::same_highlight`) is **linked**, not duplicated; its note is filled in only when empty.
* Provenance is in the **ledger** (`annotation-sync.json`: `document/annotation id` → highlight id, `KOReader (device name)`,
  the note both sides had at the last sync, `made_here`). The Notes window shows the source after the date. There is no
  `source` field on `Highlight` (that would touch 40 constructors and the catalogue format); the ledger is the single place.
* A newer note replaces the library note only while the library note is still the synced one; otherwise a **conflict** is
  counted and the library keeps its note. A deletion on the device removes a highlight only if this sync made it
  (`made_here`) and its note is untouched; a highlight the reader made in Bookx is never removed by a device. A highlight deleted
  in the library does not come back unless the device changes it later.
* **Bookmarks are imported** as real Bookx bookmarks (`Book::bookmarks`: chapter + fraction of it), see
  `src/bookmark_sync.rs` and `docs/annotation-sync-notes.md` (wave 10). The place is worked out from the device's xpointer
  (exact), a Kobo chapter file + `ChapterProgress` (exact), the chapter title, the percentage, the page number against Bookx's
  page estimate (these are rough and flagged `approx`). The ledger entry of a bookmark (`Ledger::bookmarks`, key
  `document/annotation id`) is what makes it idempotent and is also the **tombstone**: a bookmark deleted in Bookx is not
  imported again, one deleted on the device is never deleted here.
* **Bookx → device**: for books a device has synced (only those: nothing is published for other books), highlights that did
  not come from that device, are anchored and not PDF go to the server under their own id with colour/style; edits bump
  `updated`; deletions become tombstones. Highlights made on one device reach a second device's document of the same book through Bookx.

## The plugin (`koreader-plugin/bookx.koplugin/`)

* `bookxsync.lua`: all the logic, no KOReader dependency: wire format of an annotation, stable id (hash of datetime + positions,
  so editing a note keeps the id), diff against what the server last got (new/changed → send, vanished → tombstone), the
  request, `since` cursor, inbox of notes from Bookx. A failed request changes nothing; the next sync repeats it.
* `bookxjson.lua`: own JSON encoder/decoder (Lua 5.1-5.5, LuaJIT).
* `bookxmd5.lua`: own MD5, dependency-free, for the one-way hash of the progress-sync password. KOReader's `ffi/sha2` used to
  be required here; a missing FFI module made saving the password raise an error. The two implementations are compared in the tests.
* `main.lua`: KOReader glue: *Bookx Sync* menu (server and account, Sync now, Sync automatically on suspend/close, last sync
  status, "Notes from Bookx for this book"), `socket.http` / `ssl.https`. Every method and callback KOReader invokes runs
  inside `pcall`: an error is logged and shown as a message instead of panicking the reader.
* `_meta.lua`.
* **Names (bug 25):** KOReader builds the plugin with `Bookx:new{dialog, view, ui, document}` and the constructor table becomes the instance, so a
  method called `document`, `ui`, `view` or `dialog` is shadowed. Methods are `bookId`, `bookAnnotations`, `showInfo`...; only entry points
  (`syncNow`, menu callbacks, `on*` hooks) are wrapped by `guarded`, which logs `Bookx Sync error in <where>: <message>` + traceback with
  `logger.err`, keeps it as `state.last_error` (shown by *About / Diagnostics*), shows one short message (nothing for automatic runs).
  Free text is cleaned before sending (valid UTF-8, no control characters, 20 000 characters; one-line fields 1000/128/512) and requests are split
  by count (500) and bytes (600 000): the server refuses a whole request for one bad field. Details and test recipe: `docs/koreader-plugin-testing.md`.
* Installed by Share Library → *Copy to the device* (`.adds/koreader/plugins` on a Kobo, `koreader/plugins` otherwise, only
  when KOReader's folder is there; refuses to write through symbolic links) or *Save to a folder…*. The files are embedded in the binary.

## Tests

* `crates/bookx-serve/src/annotation_sync.rs` unit tests (22): auth, malformed JSON, wrong shapes, oversize, per-book cap, rate
  limit, bad document ids/traversal, idempotence, newer/older, tie, tombstones, tombstone expiry, `since`, persistence, 8 threads
  × 25 puts, 8 racing updates of one annotation.
* `crates/bookx-serve/tests/koreader_annotations.rs`: routes over a real socket, 401/413/422/400/405/404, and **the Lua plugin logic
  run under `lua`/`luajit` against the real server** (the mock `request` shells out to `curl`; skipped when `lua`, `curl` or `NET=0`
  say so).
* `koreader-plugin/tests/` (`run.sh`, details in `docs/koreader-plugin-testing.md`): `scenarios.lua` (23 stories on a fake KOReader
  that passes the plugin the same constructor arguments as `ReaderUI`; includes the bug 25 regression), `live.lua` (real `main.lua` against the real
  server, Rust test `the_real_plugin_registers_and_syncs_against_a_real_server`), CI job `koreader-plugin`, `build-aux/plugin-tests.sh`,
  `build-aux/package-plugin.sh` (`dist/bookx.koplugin.zip`). The older counts below are from before that:
* `koreader-plugin/tests/run.sh`: 54 checks of the plugin logic (JSON, wire format, diff, sync with a scripted server, failures) and
  `main_load.lua`: 28 checks of `main.lua` under a mock (33 with `BOOKX_KO_FRONTEND=<copy of KOReader's frontend/>`, which also runs
  KOReader's real menu sorter on the entry).
* Real-data checks (ignored, need a sandbox library and a copy of a device): `src/kobo_real_check.rs`.
* `src/annotation_sync.rs` (18) and `src/annotation_sync_ui.rs` (4) unit tests: merge, repeat, conflicts, deletions, unanchored
  fallback, outgoing, document matching, plugin installer (symlinks), embedded copy == files on disk.
* Broadway: `koreader_annotations`, `_conflicts`, `_refused`, `_ui` in `build-aux/ui-tests-koannot.sh`: the in-process fake
  KOReader client (`koannot:device=`) makes the plugin's HTTP calls against the built-in server on a random port.

## Sync activity and why nothing arrived (2026-11-21, plugin 0.3.1)

Owner report: "I turned on sync automatically yet I don't see the highlight I made on the e-reader; no UX shows the device connected
and what is synced". Root cause, with evidence (Kobo v2026.07.2, read-only): (1) plugin 0.2/0.3.0 crashed in `syncNow` (bug 25) until
the fixed plugin was installed at 11:02; the owner's highlight (*Saga Failure Modes*, 10:09:06, sidecar of 410 annotations) was made
in that window. (2) Even with the fix, "Sync automatically" fired only on `onCloseDocument` / `onSuspend`, never when a highlight
was made, and did nothing when `NetworkMgr:isOnline()` was false: `crash.log` at 13:00:53 `Kobo Wi-Fi: disabling Wi-Fi`, 13:01:09
`Kobo suspend`, no `Bookx Sync:` line: the suspend sync was skipped silently (KOReader turns Wi-Fi off before sleeping). (3) Nothing on
the Bookx side showed devices or results. Not the cause: the device copy of *Building Microservices* has another file than the library
(14 632 174 vs 14 676 772 bytes; partial MD5 `1029314e…` vs `13ca7995…`), but the title/author fallback matches it; the other three
books on the device have no annotations and are not in the library. Replaying the device's current sidecar (410 annotations) through
plugin 0.3.1 → server → `annotation_sync_ui::run` on a copy of the library: 407 highlights + 3 bookmarks arrive, 403 placed, 4 single
words (SOA, REST, GraphQL, DRY) kept unplaced, and the 10:09 highlight is placed (yellow, correct text).

What 0.3.1 / this change adds:

* Plugin: `onAnnotationsModified` (30 s debounce), `onReaderReady`, `onNetworkConnected`; an offline automatic run records `state.waiting`
  (menu line, diagnostics) instead of nothing and never switches Wi-Fi on; `X-Bookx-Device`, `X-Bookx-Device-Id`, `X-Bookx-Client`
  (`KOReader/<ver> bookx-plugin/<ver>`), `X-Bookx-Protocol` on every request (ASCII only; the body keeps the real device name); the
  answer of a PUT carries `in_library` (true/false/null), the plugin lists `false` ones in diagnostics and warns after *Sync now*;
  counts sent/received are kept in `state.stats`. Old plugins send no headers: the server uses the body's `device`/`device_id`, else
  "Unknown device (KOReader)". No protocol bump (additive).
* Server (`bookx-serve/src/activity.rs`): `ActivityLog` (devices: first/last seen, last sync, client, plugin, protocol; 200 events;
  file `sync-activity.json`; `AnnotationSync::with_activity`, `KoSync::with_activity`). Only authenticated requests are recorded
  (a wrong password leaves no trace on purpose); refused authenticated requests are `Rejected` events; progress PUTs are `Progress`
  events (also from KOReader's own Progress sync).
* App: `merge_incoming` returns per book/device deltas; `record_merge` writes `NotInLibrary` (once per change of count) and `Unplaced`
  events, `Ledger::unmatched` (listed under *Books not found in your library*), `Ledger::last_synced` (book details, row *Sync*) and tells the
  server which documents are in the library. UI: `src/sync_activity.rs` (text, injectable `now`), `src/ui/sync_activity.rs` (groups in the Sync
  window and Preferences → Sync), toast with *Show* (`annotation_sync_ui::announce`, library hooks `sync_activity::set_hooks` → `Input::Refresh`,
  `Input::ShowNotesFor`). Dev: `sync:activity`, `koannot:toast`; Broadway `koreader_annotations_activity`.
* Not done / caveats: the *Show* button narrows Notes & Highlights by the book title (not by device and time); a device that talks only
  once shows "Last seen …" until opened again (the status is computed when the window refreshes, every 3 s); failed logins are not listed;
  Wi-Fi is never switched on by the plugin (an opt-in "turn on Wi-Fi to sync" would be a separate decision); the first run of 0.3.1 on
  the device is unconfirmed.

## Colour and style (2026-11-21)

Full analysis: `docs/highlight-sync-rca.md`. In short: `color` and `drawer` travel exactly as KOReader writes them (the plugin sends the
word verbatim, control characters removed, 32 characters at most: the server refuses a longer value and with it the whole request); Bookx
stores them through `bookx_core::devstyle`, the one table for the USB path, this path, the exports and the notes file written for KOReader.
There is no protocol change: the released plugin keeps working against this Bookx, and this plugin against an older Bookx (which would read
a drawer it does not know as the default). Look tests: `devstyle` (every colour and drawer, both ways, Kobo 0..3 and `kobo-<n>`),
`annotation_sync::a_colour_and_drawer_nobody_knows_arrive_as_written_and_a_repeat_changes_nothing`,
`::what_bookx_sends_carries_the_nearest_koreader_colour_and_drawer`, `::a_spelling_of_the_same_look_is_not_a_change_to_send`,
`device_sync::a_short_quote_inside_a_longer_highlight_does_not_take_its_colour`, `duplicates::merging_keeps_the_colour_and_style_of_the_copy_that_has_them`,
`import::the_same_look_in_two_spellings_is_one_highlight`, `bookx-serve/tests/highlight_looks.rs`, `look_repair.rs`, the Lua checks of the wire format.

## Live updates (2026-11-21, wave 11)

Owner report: "The sync works, but the reader view doesn't update in real time." Before: the merge ran from a 4 s timer and only the
library window was told; a book that was open kept the highlights it had loaded.

**Path.** Server worker thread: `bookx_serve::notify` (`Notifier`, `Note::{Annotations,Progress}`; `AnnotationSync::on_change`,
`KoSync::on_change`; called after the store is written and unlocked, once per request that changed the store, never for Bookx's own
`origin:"bookx"` publishing, never for a repeat or a stale version) → `sync_bus::server_listener()` pushes the note into a `Mailbox`
and wakes the main loop with `MainContext::invoke` (no GTK off the main thread; cap 1024 notes, oldest dropped, every note only means
"look again") → `annotation_sync_ui::install_live` plans (`sync_bus::plan`: one merge for any number of annotation notes, the last
position per document and device), waits 250 ms (`DEBOUNCE_MS`) and runs the normal `sync_once` → what changed is found by comparing
`Snapshot`s of all books before and after (hashes of highlights/notes/looks/bookmarks/progress), so the Wi-Fi merge, the USB sync
(`ui/devices.rs`) and imports (`ui/annotations.rs`) publish identical typed `SyncEvent {book_id, kinds, device, via, ahead}` →
`sync_bus::publish` coalesces per (book, device) (`Coalescer`: 250 ms quiet or 1 s at most) → subscribers on the main thread:
each open reader (`subscribe(Some(book))`), the library window (`subscribe(None)`). A reader that closes drops its `Subscription`.
Total latency from the PUT: about 250 ms merge debounce + about 50-300 ms event window + the repaint, in Broadway about 0.6 s.

**Reader** (`ui/reader.rs::on_sync`, `reader_pdf.rs::on_sync`, `reader_comic.rs`): `sync_live` is the pure logic (`diff` of highlight
signatures → `draw`/`removed` lists, `repaints` only when the chapter on screen is touched, `note_action`, `Live` banner rules, `worth_offering`).
The highlights come from the in-memory library the merge already updated, so there is nothing to load off-thread; the sidebar lists
are rebuilt only when something changed. The book map updates in place (`reader_map::update_live`). The banner is `ui/reader_live.rs`
(one widget for the three readers, in the page overlay). The reading position is **never** applied: a `Note::Progress` is resolved
to a book by the partial MD5 in the ledger (`resolve_docs`, digests cached), becomes an event with `ahead`, and the reader offers
"Continue There" only if the device is more than 1 % ahead of the reader (`kosync_sync::ASK_GAP`), is not this installation
(`kosync.device_id`) and the switch `share.offer_jump` (Preferences → Sync, default on) is on. The library's own progress is not touched,
so "progress only moves forward" holds trivially; Continue There moves the page and the normal progress save then raises it.
**Library window**: `Input::SyncLive` refreshes counts, the Notes view (`reload`, scroll position kept by
`ui::sync_activity::keep_scroll`), the card when the library position changed (`requery`, selection kept) and the details panel
(skipped while it has the keyboard focus). The sync toast keeps *Show* and adds *Open Book* when no reader of that book is open.

**Tests.** Unit: `bookx-serve` (`notify`, `annotation_sync::...notifies_exactly_once...`, e2e over localhost
`a_plugin_style_put_over_localhost_notifies_the_listener_once_per_merge`, `a_reading_position_over_localhost_notifies_once`),
`sync_bus` (coalescing, order, burst, snapshots, mailbox from 8 threads, drop safety, plan), `sync_live` (diff, note conflict,
banner rules). Broadway `koreader_annotations_live_*` (reader, burst, bookmarks, library, conflict, conflict_keep, ahead, ahead_dismiss,
ahead_off) use `koannot:push|pushdel|progress` (real HTTP PUTs, no merge call), `reader:live-state|live-edit|live-type|live-press`,
`koannot:events`.

**Limits / not done.** The merge itself still runs on the main thread (as before); 410 annotations arriving at once is a visible pause,
a handful is a few milliseconds. The e-reader side is not real time: plugin 0.3.1 sends ~30 s after a change. A device whose KOSync
document id is the file-name hash (not the partial MD5) is not matched for the position banner. The comic reader only gets the position
banner (comics have no highlights or bookmarks in the library). Real GNOME session scenarios were added on 2026-11-22 (`live_sync_highlight`, `live_sync_ahead_banner`, `sync_activity_window` in `build-aux/session/scenarios_wave11.py`);
untested with a real Kobo. Strings added (no po edits): see the change description.

## Not verified / caveats

* **First device run, 2026-11-21 (bug 25):** *Test connection* worked on the owner's Kobo; *Sync now* failed with the shadowed `document` method;
  fixed in plugin 0.3.0, **not yet confirmed on the device after the fix**. What Bookx writes for KOReader: nothing in the `.sdr`; notes made in Bookx reach
  the device only through the plugin's inbox (*Notes from Bookx for this book*), so KOReader never has to read a notes file. The real sidecar layout
  (annotations table with `color`/`drawer`, `datetime_updated`, bookmarks without `drawer`) matches the fixtures and `devstyle` (test
  `every_look_word_seen_on_a_real_kobo_is_known`); `gray` is in KOReader's menu but not in its colour table, so a gray highlight shows with KOReader's fallback.
  Times: KOReader writes local time without a zone; the plugin converts with the device's `os.time`, so a Kobo whose timezone is not set sends UTC-based times.
* (history) **Never run inside KOReader, on a Kobo, Kindle or any device.** Since 2026-11-20 the KOReader API use was checked by reading the frontend
  of the owner's KOReader v2026.07.2, and `main.lua` runs under host luajit with the real `luasettings`, `dump`, `gettext`, `logger`,
  `ffi/sha2` and `menusorter` of that version (`koreader-plugin/tests/main_load.lua`; the widgets and `socket.http` are mocks).
  See `docs/kobo-koreader-sync-runbook.md` and bug 15. Real HTTP from KOReader's `socket.http`, the touch UI and the Wi-Fi path are not verified.
* **The device does not paint Bookx's highlights.** The plugin receives them and lists them (*Notes from Bookx for this book*);
  creating KOReader highlights from text would need a text search in KOReader's engine that was not attempted.
* KOReader deletions on one device are not applied to other KOReader devices by the plugin (Bookx is the hub and applies them to
  its library only when it made the highlight).
* Document match by partial MD5 fails when the device copy is not byte-identical to the library file (a converted or KEPUB
  copy); then the title/author fallback is used.
* No QR code (no QR dependency in the tree).
* The plugin talks plain HTTP or HTTPS with KOReader's certificate store; a self-signed Bookx certificate will not be trusted by
  KOReader. Use HTTP on a trusted network.
* The Lua test exercises the real wire format through `curl`, not through KOReader's `socket.http`.
