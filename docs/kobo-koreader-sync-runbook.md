# Kobo + KOReader sync: runbook and verification record (2026-11-20)

Internal. For the user text see `wiki/docs/koreader-sync.md`; design in `docs/koreader-annotation-sync.md`.
Device: the owner's Kobo with KOReader **v2026.07.2** in `.adds/koreader`, books and `.sdr` folders at the top level.

## 1. What was verified here, and what was not

| Item | How | Status |
|---|---|---|
| Plugin API calls against KOReader's own source | read `frontend/`, `kosync.koplugin`, `wallabag.koplugin` on the device (bug 15 in `docs/bugs.md`) | verified by reading |
| `main.lua` executes (init, menu, settings dialog, sync, inbox, events) | host luajit + real `luasettings`/`dump`/`gettext`/`logger`/`ffi/sha2`/`menusorter` from the device copy; widgets, `socket.http`, `NetworkMgr` are mocks (`koreader-plugin/tests/main_load.lua`, 96 checks with the real modules, 74/82 logic checks) | verified under a mock |
| Plugin folder installed on the device | the old copy on the Kobo was missing `bookxmd5.lua`; the full seven-file installer was copied to `.adds/koreader/plugins/bookx.koplugin/` on 2026-11-20 and the Lua suite was rerun against the device's modules (0 failures) | installed, not yet pressed |
| The plugin pressed on the device | needs fingers on the Kobo (Register account, Test connection, Sync now, the old panic path and coloured highlights) | **NOT verified: owner** |
| Real `.sdr` / Kobo database vs what Bookx imports (USB path) | `src/kobo_real_check.rs` (`kobo_real_check`, ignored) on a sandbox copy of the real library | verified, see section 2 |
| Wireless path with the real sidecars | `kobo_real_wireless_check`: plugin Lua (`bookxsync.lua`, curl transport) -> built-in server on a random port -> merge into the sandbox library -> back to the plugin | verified, see section 3 |
| KOReader's own HTTP stack (`socket.http`, LuaSec) talking to Bookx on the LAN | cannot be run here | **NOT verified: owner** |
| Wi-Fi from the Kobo to this computer, firewall | no | **NOT verified: owner** |

The Kobo's own database `KoboReader.sqlite` has **0 rows in `Bookmark`** (the owner reads in KOReader), so there was no Kobo-native
highlight to compare. All highlights are in the KOReader sidecars.

## 2. Accuracy of the USB import (real data)

Ground truth: the four `.sdr` files (only *Building Microservices* has annotations: 409 = 406 highlights + 3 page bookmarks;
no notes; `lighten` 382, `underscore` 24; yellow 282, purple 56, cyan 43, olive 10, red 6, orange 5, green 4).
The library copy already held 400 of them from an earlier import.

Field by field after the fixes (`/tmp/opencode/kobo/compare.py`, sandbox copy of `~/Documents/Bookx Library`):

* text: 406 of 406 identical to the device text;
* colour: 406 of 406 (the 123 that an earlier import had stored as default yellow are corrected, cyan/olive kept as written);
* style (drawer): 406 of 406;
* notes: none on the device, none invented;
* placement (text of the painted range equals the device text): 402 of 406. The 4 others are the single words **SOA, REST, GraphQL, DRY**:
  `anchor::MIN_QUOTE` is 8 characters because a shorter quote matches many places, and Bookx never guesses a place, so they are kept
  as notes without a place (*Could not be located in the book*). The device's xpointer would place them but is deliberately not used
  (the text decides). Seven painted ranges differ from the device text only by a no-break space (same words);
* bookmarks: the 3 page bookmarks are counted as "left out" (Bookx does not import KOReader page bookmarks); they used to come in as
  bogus highlights (bug 13);
* duplicates: none; the term *Request-response* highlighted twice (pages 150 and 157) is two highlights at two places;
* timestamps: the day matches for 406 of 406; the **time of day is the device's local time stored as UTC** (KOReader writes no zone),
  so Bookx shows it 5 h 30 min early in India. Not changed (the sidecar says nothing about the zone). 168 older highlights carry the edit
  time instead of the creation time (imported by the older code); new imports use the creation time.

## 3. Accuracy of the wireless path (real data)

Replay of the four real sidecars through the plugin logic to a server on a random local port with annotation sync on, merged into a sandbox copy:

* the plugin sent 409 annotations (406 highlights + 3 bookmarks) for *Building Microservices*; a second run sent 0 (diff works);
  *Practical MLOps* (two authors) failed before the fix (`authors` had a newline), works now;
* the server stored them with the right colour, drawer, chapter, `pageno`, datetime; the merge linked 402 and kept 4 as notes without a place;
  a second merge changed nothing;
* Bookx -> device: a highlight made in Bookx (blue, underline, note) was published and arrived in the plugin's inbox
  ("received 1"); nothing from the device bounced back (it was 23 before the fix).

## 4. What is installed on the device

Only `.adds/koreader/plugins/bookx.koplugin/` (files: `_meta.lua`, `main.lua`, `bookxsync.lua`, `bookxjson.lua`, `bookxmd5.lua`, `LICENSE`, `README.md`),
written with the same rules as the app's installer (`src/annotation_sync_ui.rs::install_plugin`: temp file then rename, symlinks refused,
nothing deleted). Backup of everything that could have been touched: `~/.cache/kobo-backup-20261120/` (all `*.sdr`, `settings/`,
plugin list, `KoboReader.sqlite`, `metadata.calibre`, with SHA-256 compared against the device). The script is
`/tmp/opencode/kobo/install-plugin.sh` (see the report for whether the device was mounted when it ran).

## 5. What the owner does

On the computer (Bookx):

1. Start Bookx with the new build (`./run.sh`). Library opens; the one-time highlight repair runs.
2. **Share Library**: switch on *Share this library*, *Allow other devices on my network* (set a password), *KOReader progress sync* and
   *Sync highlights and notes*. Note the address it shows. Default port is 8080: this computer is `192.168.1.162` (Wi-Fi `wlp12s0`), so the
   server address for the Kobo is **`http://192.168.1.162:8080/bookx`** (check the number in the window, the IP can change; use HTTP, not HTTPS).
3. Firewall: `firewalld` is running (zone `FedoraWorkstation`) and it already allows **TCP 1025-65535**, so port 8080 is open on the LAN.
   Nothing to change. If you pick a port below 1024 or the Kobo cannot connect, run `sudo firewall-cmd --add-port=8080/tcp` (temporary).
   Both devices must be on the same Wi-Fi network and the router must not isolate clients ("AP isolation" off).

On the Kobo:

4. Eject the Kobo from the computer in the file manager (do not just unplug). Let the Kobo restart into its normal screen.
5. Start KOReader (Kobo home screen: *KOReader* / the usual way you open it). Plugins are loaded at start: if KOReader was already running, restart it
   (Menu → exit, open again).
6. Open a book (e.g. *Building Microservices*). Tap the top of the screen for the menu. Tabs: pick the **Tools** tab (wrench icon) →
   **More tools** → **Bookx Sync**. (If it is not there: Tools → *More tools* → *Plugin management* and check "Bookx Sync" is listed and enabled.)
7. **Server and account**: server `http://192.168.1.162:8080/bookx`, user name, password (the same account as *Progress sync*; create it with
   *Tools → Progress sync → Custom sync server* + *Register* if you have none), optionally a name for the Kobo, **Save**.
8. Turn Wi-Fi on (KOReader asks when needed). **Sync now**. A message "Sent N, removed 0, received M from Bookx" means it worked; an error message says why
   (wrong user name or password, "the Bookx server has no KOReader sync (switch it on in Share Library)", timeout = firewall/IP).
9. Optional: **Sync automatically** (syncs when the book closes and when the Kobo sleeps).

Check in Bookx afterwards:

* Notes & Highlights (or the book's highlights): the highlights of *Building Microservices* are there once each (406), the source reads
  "KOReader (device name)". Four single words (SOA, REST, GraphQL, DRY) show as *Could not be located in the book*; that is expected.
* Make a highlight with a note in Bookx, wait a few seconds (the window's sync line), press **Sync now** on the Kobo, open
  *Bookx Sync → Notes from Bookx for this book*: it is listed.
* Make a highlight on the Kobo, **Sync now**: it appears in Bookx within a few seconds.
* If anything fails, `~/.cache/kobo-backup-20261120/` holds the original sidecars; the plugin never writes to a book, a `.sdr` or KOReader's settings
  (only `settings/bookx_sync.lua` and `settings/bookx_sync_state.lua`).

To remove the plugin: delete the folder `.adds/koreader/plugins/bookx.koplugin` (and the two `settings/bookx_sync*.lua` files) from the Kobo.
