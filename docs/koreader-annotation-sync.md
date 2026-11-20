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
* Bookmarks are remembered (counted) but **not imported**: a Bookx bookmark needs an exact place and KOReader gives a
  page in its own layout.
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
* Installed by Share Library → *Copy to the device* (`.adds/koreader/plugins` on a Kobo, `koreader/plugins` otherwise, only
  when KOReader's folder is there; refuses to write through symbolic links) or *Save to a folder…*. The files are embedded in the binary.

## Tests

* `crates/bookx-serve/src/annotation_sync.rs` unit tests (22): auth, malformed JSON, wrong shapes, oversize, per-book cap, rate
  limit, bad document ids/traversal, idempotence, newer/older, tie, tombstones, tombstone expiry, `since`, persistence, 8 threads
  × 25 puts, 8 racing updates of one annotation.
* `crates/bookx-serve/tests/koreader_annotations.rs`: routes over a real socket, 401/413/422/400/405/404, and **the Lua plugin logic
  run under `lua`/`luajit` against the real server** (the mock `request` shells out to `curl`; skipped when `lua`, `curl` or `NET=0`
  say so).
* `koreader-plugin/tests/run.sh`: 54 checks of the plugin logic (JSON, wire format, diff, sync with a scripted server, failures) and
  `main_load.lua`: 28 checks of `main.lua` under a mock (33 with `BOOKX_KO_FRONTEND=<copy of KOReader's frontend/>`, which also runs
  KOReader's real menu sorter on the entry).
* Real-data checks (ignored, need a sandbox library and a copy of a device): `src/kobo_real_check.rs`.
* `src/annotation_sync.rs` (18) and `src/annotation_sync_ui.rs` (4) unit tests: merge, repeat, conflicts, deletions, unanchored
  fallback, outgoing, document matching, plugin installer (symlinks), embedded copy == files on disk.
* Broadway: `koreader_annotations`, `_conflicts`, `_refused`, `_ui` in `build-aux/ui-tests-koannot.sh`: the in-process fake
  KOReader client (`koannot:device=`) makes the plugin's HTTP calls against the built-in server on a random port.

## Not verified / caveats

* **Never run inside KOReader, on a Kobo, Kindle or any device.** Since 2026-11-20 the KOReader API use was checked by reading the frontend
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
