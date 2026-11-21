# Real Kobo device report (internal)

Date: 2026-11-12. The first time Bookx's Kobo code met a real Kobo. The owner's reader, mounted read-write at
`/run/media/<user>/KOBOeReader` (vfat, `uid=1000`, `flush`, `errors=remount-ro`). Serial number deliberately not
recorded here.

## Device

| | |
|---|---|
| Model | Kobo Libra Colour (`.kobo/version` model id `…0390`; `driveinfo.calibre` says the same) |
| Firmware | 4.46.23852 (kernel 4.9.77) |
| Database | `.kobo/KoboReader.sqlite`, **schema version 176** (`DbVersion`), **WAL journal mode** (file header bytes 18-19 = 2), page size 1024, 3.8 MB |
| Storage | 29.0 GB vfat, 692 895 744 bytes used, 28 328 968 192 free; 16 KiB clusters |
| Other software on it | KOReader v2026.07.2 under `.adds/koreader` (started from NickelMenu, `.adds/nm`), Calibre (`driveinfo.calibre`, `metadata.calibre`), macOS leftovers (`._*`, `.Spotlight-V100`, `.fseventsd`, `.Trashes`) |

## How the owner's books are stored

* 6 EPUB files directly in the root of the volume (no `Bookx/` or other folder), copied by Calibre, **none of them
  imported by the Kobo firmware**: the `content` table has no row for them.
* Their reading state lives only in **KOReader sidecars** `<name>.sdr/metadata.epub.lua` (4 of the 6 have one; one has
  269 `annotations` with notes, written in the modern `annotations` format; each has `percent_finished`,
  `summary.status`, `summary.modified`).
* The database has 1 037 rows of content type 6: **940 are KOReader's own files** (825 gif, 104 svg, 6 png, 4 txt,
  1 html below `.adds/koreader`: the firmware indexes everything it finds) and 97 are books by MIME type: 72 store
  previews (UUID ids, `IsDownloaded` false, `application/x-kobo-epub+zip`), 21 `application/epub+zip` (20 KOReader
  news EPUBs below `.adds` and 1 sideloaded EPUB whose file no longer exists) and 4 UUID `octet-stream` rows.
  1 203 chapter rows (type 9) have ids like `file:///…epub#(0)OEBPS/content.html` or the odd form
  `…gif//mnt/onboard/…`. No shelves, no highlights (`Bookmark` empty), 12 `Event` rows.
* Content ids are `file:///mnt/onboard/<path>` with the raw (not percent-encoded) path, as the code assumed.

## Findings: code against the real device

| # | Assumption in the code | Reality | Action |
|---|---|---|---|
| 1 | `kobo::read_states/read_shelves` opened the database on the device with a read-only SQLite connection and called that "read-only". | The database is in WAL mode. A read-only SQLite open of a WAL database **creates/updates `-shm` and `-wal` next to it** (reproduced on a scratch copy: a clean one-file WAL database gains `-shm` and `-wal` after a `mode=ro` open and keeps them). On the device `KoboReader.sqlite-shm` carried a modification time of 16:36, ten minutes before this study started, while the Bookx application (a release build) was running. The Bookx device window is the likeliest cause. Harmless for the data here (the `-wal` is 0 bytes, the main file is untouched since 12:27) but it is a write to the user's reader. | **Fixed.** Reads copy the database (and a left-over `-wal`) to a private temp folder and read the copy; nothing on the device is opened by SQLite. Regression tests: `reading_a_wal_database_leaves_the_folder_exactly_as_it_was`, `changes_still_in_the_wal_are_seen`, `device_check` test `…reports_model_firmware_and_database_facts` (asserts `.kobo` listing unchanged). |
| 2 | `backup_once` copied only the main file. | With WAL, committed changes may live only in `-wal`, so the backup could miss them. | **Fixed.** The backup is made with `VACUUM INTO` from the private copy (includes the log). Test: `shelf_writes_on_the_real_schema_only_add_rows` (backup has the same rows as before the change). |
| 3 | Rows of content type 6 are books. | 940 of 1 037 are KOReader's assets; 72 are store previews. | **Fixed.** `is_book_mime` filters images/text/scripts; `rel_of_content_id` already ignored previews (UUID ids). Test: `real_rows_give_books_not_the_noise_below_adds`. |
| 4 | Reading state comes from the database (and from KOReader as fallback). | For this owner it comes **only** from KOReader: the firmware never imported the books. The progress of the sideloaded books is invisible to the old code unless KOReader sidecars parse. | Sidecars are now the first-class source. `device-check` warns "6 of 6 book files are not in the Kobo database". |
| 5 | Sidecar parsing by searching the text for `["percent_finished"]` / `["status"]` / `["title"]`. | Real sidecars begin with the annotations; a highlight whose text quotes those keys would be taken for the setting, and the 208 KB file is searched three times. No last-read date was read. | **Fixed.** New `lua.rs` data parser (no code execution, depth bounded) and `reading::parse_sidecar` read the structure: progress, status, title, `summary.modified` as last-read date, and the **annotations** (text, note, chapter, dates). The old key scan remains as fallback for files that are not valid Lua. Tests: `a_real_shaped_sidecar_is_read_by_structure…`, `lua::tests`, env-gated `real_koreader_sidecars_are_parsed_completely` against the four real sidecars. |
| 6 | Highlights were not read from the device at all. | Kobo keeps them in `Bookmark` (empty here), KOReader in the sidecar (266 on one book). | New `kobo::read_notes` (`Type` highlight/note, hidden rows skipped) and sidecar notes, both feeding `reading::collect_data`. |
| 7 | `device-check` looks only in `Bookx/`. | The owner's books are in the root, so a check of this device saw 0 books and said "folder does not exist yet". | `device-check --all` checks every book on the device. `device-check` also prints model, firmware (from `.kobo/version`, serial not printed), free space, schema version, journal mode and counts. It was already strictly read-only (no `--read-only` flag needed); it now reads the database through a copy. |
| 8 | KEPUB check = "some file contains koboSpan". | Needed for the sandbox verification. | `device-check` now also parses every span document as XML, requires unique `kobo.<para>.<sentence>` ids (FAIL on duplicates/malformed XML) and the `book-columns`/`book-inner` wrapper (warn). |
| 9 | Hidden entries are skipped. | Right: the root is full of `._*.epub` AppleDouble files that look like books by extension; they are skipped (`name.starts_with('.')`) in `list_books` and `device-check`. `.adds`, `.kobo`, `.sdr` are skipped as well. | none |
| 10 | Detection by `.kobo` folder; main memory by volume name; free space by `statvfs`. | Confirmed (`device kind: Kobo`, `28.3 GB free of 29.0 GB`). | none |
| 11 | The sent book goes to `Bookx/`, optional shelf entry written to the database. | `Bookx/` does not exist on this device (it is created on the first send). Sending itself works (below). **Whether the firmware keeps a `ShelfContent` row for a file it has not imported yet is unknown**: Calibre's driver writes shelves only after the device indexed the book. | Left as is, opt-in (empty shelf name by default). Listed as unverified. |
| 12 | KOReader sidecar written by *Send … with Notes* uses the old `bookmarks` key. | The device's KOReader (v2026.07.2) stores `annotations`. Reading the KOReader source on the device (`readerannotation.lua`): when `annotations` is absent it migrates `bookmarks` (page-type bookmarks without `highlighted` become plain bookmarks), so the old format is accepted. Not run on the device. | Documented; Bookx now reads both keys. |
| 13 | `rel_of_content_id` drops ids containing `!`. | Real chapter ids use `#(n)` and `//mnt/…`, but only type 9 rows, which are never read. | none |

## Sync plan (enhancement 4, backend part)

`bookx-serve/src/syncplan.rs`, pure functions. Rules: nothing is ever deleted or lowered (the plan type has no removal
variant); progress goes to the side that is behind; *Finished* is never undone; a side that is behind but was read
more than a day later is reported, not overwritten; highlights existing on one side only are added to the other; the
same highlight on both sides takes the richer text/note; two different notes on the same highlight are both kept and
reported as a conflict. `SyncPlan::totals()` gives the counts, `describe()` the sentence for the prompt. Applying a
plan and planning again yields an empty plan (test `applying_a_plan_makes_the_next_plan_empty`).
`devices::full_sync_plan(drivers, device, books, library)` in `src/devices.rs` builds it from a connected device and
the library; the Device window (UI agent) still has to call it and apply the result. Applying device-side changes is
not written: only the plan exists.

## What was verified on the real device

Phase 1 (read-only, `BOOKX_KOBO_SNAPSHOT` copies in `~/.cache/bookx-kobo-snapshot/`):

* `bookx-cli device-check <mount> --all` on the real mount: exit 0, model/firmware/schema/counts as above, the six
  EPUBs are valid containers.
* Database read, schema, shelves, notes, `read_states`, `db_info`, **shelf write with backup and `integrity_check`
  on a copy**, `Bookmark`/`content` unchanged by the shelf write, snapshot itself byte-identical afterwards
  (test `kobo_snapshot.rs`).
* The four real sidecars parse completely (progress, status, date, 266 annotations).

Phase 2 (writes, only below `/run/media/<user>/KOBOeReader/BookxSandbox/`, started 16:58, finished a few seconds later):

* Public-domain test books (`docs/test-books.md`) were sent through `devices::send_full` (the app's own path:
  EPUB → KEPUB conversion, save-path template, `unique_name`, `write_new_file` with `sync_all`) using a `DeviceInfo`
  whose mount was the sandbox folder; the shelf option stayed off, so the Kobo database was not written. One book was sent
  *with Notes* (appendix chapter + KOReader sidecar).
* Results (read back from the device): `Alice's Adventures in Wonderland.kepub.epub` 200 079 bytes, sha256
  `5910463238e949c66589988aab80ea0024ba64baff35eb4403a168a0a01fa151`; a second copy sent with exactly the bytes the
  test converted itself is identical (so the write path is byte-exact); `The Masque of the Red Death.kepub.epub`
  103 430 bytes, sha256 `9f6432847662a90ed3227bd5f9138b4d709c929998cddda8641840114bb59392`, with
  `OEBPS/bookx-annotations.xhtml` and a 430-byte `…kepub.sdr/metadata.epub.lua`. The device copies hashed the same
  after a `sync` and after copying them back to the computer.
* Structure: ZIP test clean, `mimetype` first and stored; 15 and 4 XHTML documents all well-formed; 2 386 and 295
  `koboSpan`s with unique `kobo.P.S` ids; the text of every document, with spans removed, equals the original text
  (compared per file with tags stripped and white space removed); `book-columns`/`book-inner` wrappers present;
  names are legal on vfat (apostrophe, spaces; 43 and 38 bytes); free space dropped by 606 208 bytes (clusters) and
  was back to the byte afterwards.
* `device-check --kind kobo` on the sandbox: every book "valid EPUB container" and "KEPUB markup present"
  (the only FAIL is `.kobo folder missing`, expected because the sandbox is not a device root).
* Cleanup: `BookxSandbox/` removed; `sync`. **sha256 of all 5 399 files on the volume identical before and
  after** (`sha-full-before.txt` vs `sha-full-after.txt`, diff empty); the full listing with sizes and modification
  times (6 024 entries, dirs included) identical; free space 28 328 968 192 bytes, identical to the start.

Hashes of the files that matter (before = after phase 1 = after phase 2):

```
cb55da86767b5e884a57e7eeb8b75fbcb354641bb4e06ea83fbca455dad9740e  .kobo/KoboReader.sqlite
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  .kobo/KoboReader.sqlite-wal   (empty file)
f0220c8ec1638cb0861e3ddfb92335b4eb6e4e8e99ba85f5a76085b8621e7608  .kobo/version
e5a25207f1a9f52889eff53c561d0d18532e330ac38951f22e657d26695331ce  .kobo/BookReader.sqlite
3c2bb4e2c47c3eb1db38f0ad61fb1546860c8036a43a208ec0dc54ab0e0529c8  metadata.calibre
3f0ee50c5b68d38a9f698d2022ff724c1686540dcc14e0014c9f6ee21979d2a9  driveinfo.calibre
(sha256 of the six owner EPUBs in the volume root omitted from this public copy; they were identical before and after)
```

(`KoboReader.sqlite-shm` is not hashed: it is a scratch file that is rebuilt on open; its size 32 768 and mtime
16:36:08 are unchanged by this study.)

## What was NOT verified

* Nothing was ejected or unplugged (not allowed), so the **Kobo firmware never saw the sandbox books**: whether it
  imports them, shows covers/title/author, shows the reading-stats line for the KEPUBs, opens them, and whether it
  accepts a sidecar-less KEPUB with the notes appendix were not tested. B3-B9 of `hardware-test-plan.md` are still open.
* Shelf writing against the real device. Only a copy of the database was written. Whether the firmware accepts or
  discards a `ShelfContent` row for a not yet imported book is unknown (finding 11).
* The KOReader sidecar written by *Send with Notes* was not opened in KOReader (compatibility is derived from the
  KOReader source on the device, finding 12).
* Applying a sync plan to the device (writing progress/highlights to KOReader or the Kobo database) is not implemented.
* Other models and firmware: only a Libra Colour on 4.46.23852. The model table in `kobo::model_of` knows only this
  id; any other shows its raw id.
* A database that is not cleanly closed (non-empty `-wal`): handled by the copy-with-log path and a test, never
  seen on this device.

## Process notes and anomalies

* Phase 2 started without an external review of phase 1 (the subagent could not ask for one); the gate was replaced
  by a self-check (hashes and listing identical after phase 1) and by writing only inside the sandbox.
* The modification times of `KoboReader.sqlite-shm` (16:36) and `-wal` (15:55) and of `driveinfo.calibre` /
  `metadata.calibre` (15:54) are earlier than this study; the cause is not Bookx's study commands (hashes and listing
  taken at the start show the same values at the end). Finding 1 explains how the running application could do it.
* While the study ran, another agent was editing `src/devices.rs` and `src/ui/*`; the root crate did not compile for
  a while because of their in-progress work. One import in `src/devices.rs` flipped between two forms; it is now
  `#[cfg(test)]`-gated.
* The apostrophe in `Alice's Adventures…` and the Unicode-free names were accepted by vfat as is. `sanitize_file_name`
  was not changed.
