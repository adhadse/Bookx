# Device UI: notes for developers

Enhancement 4 and bugs 3 to 5 (see `docs/enhancements.md`, `docs/bugs.md`), plus the wave 7 Device UX redesign
(`docs/human_interface_backlog.md` §7 D2/D5/D7). Public description: the wiki pages *Library* and *Devices and sharing*.
This file is internal: decisions, component map and dev hooks, not user copy.

## Pieces

| File | What |
|---|---|
| `src/device_sync.rs` | pure: `plan` (device readings + library books → what would change), `Plan::summary` (counts and words), `apply_book` / `apply_to_library` (library side), `apply_to_device` through the `NoteSink` trait (`SidecarSink` writes a new KOReader sidecar, never over an existing file), `Locator` trait (finds a device quote in the book; `BookLocator` searches the chapter text), `read_device` (glue over `bookx_serve::reading::collect_data`), `SyncPrefs`, header-button and banner texts |
| `src/device_covers.rs` | cover bytes of EPUB/KEPUB device books, cached in `<cache>/device-covers/` keyed by path, size and mtime (a book without cover is remembered with a `.none` file) |
| `src/ui/devices.rs` | the **Devices** window (plural): two pages, **Books** and **Settings**; the Books page has the individual **Device** row, *Books on the device* (covers, hover/selection checkboxes, one grid/list toggle), *Books to send*, `start_sync` (read → plan → summary dialog or automatic apply) and `send_books`; Settings holds Sending options, Sync and Cloud folder (WebDAV) |
| `src/ui/library.rs` | **Devices** button, the `adw::Banner` **E-reader connected** with one-shot **Sync** and built-in dismiss, device scan every 3 s on a worker thread, details panel opening, breakpoint, News in the menus |

## Rules of the sync (all enforced and tested in `device_sync.rs`)

* progress moves forward only (the furthest device copy wins; finished is 100 %); a library that is ahead is reported,
  the device is never written to for progress;
* highlights are the same when the id matches or the text matches without case, spacing and punctuation;
  new ones are added to the side that lacks them, never removed; a note fills an empty note but never replaces one;
* a device copy that already has a KOReader notes file is never written to (counted as *held back* in the summary);
* the connection banner's **Sync** is one-shot and never changes settings. Persistent sync is only the **Always sync when
  connected** switch in Devices → Settings, **off by default**.

## Caveats

* `src/devices.rs` still holds the Kobo agent's `bookx_serve::syncplan`-based `full_sync_plan`; the UI uses its own
  plan in `device_sync.rs` because it has to place incoming highlights in the book (locator), know which device copies
  can take notes and apply to the library. The two should be merged one day (the serve plan also reports conflicts and
  "behind but newer").
* Incoming highlight positions come from searching the chapter text (`Doc::to_text`) and are character offsets in
  that text; they are exact only when the reader's text is the same. The quote text is always kept.
* Not tried on real hardware; the scenarios `device_ui`, `device_sync_ui`, `device_auto_sync`, `device_window_hig` and
  `device_fit` use simulated devices (`BOOKX_DEVICE_ROOTS`). The real-session a11y scenarios are `device_window_a11y`
  and `device_banner_a11y`.
* The window title and hamburger entry are **Devices**; only an individual row is titled **Device**. The Books page shows
  **No Device Connected** when nothing is plugged in. Do not reintroduce the old *Always*/*Not now* banner answers.
* The Devices dialog keeps its content under 560 px high and at most 85 % of the parent window (`device_fit`), so the
  send controls stay reachable at 900x600.
* No drag and drop of library books onto the Devices button.

## Dev hooks

`library:devices-scan`, `library:forget-devices` (then `devices-scan` behaves like a new connection),
`library:device-banner=dismiss` (the canonical hook; `sync` runs the one-shot summary, old `always|later` aliases only
dismiss and never turn auto-sync on), `library:device-button`, `library:autosync=on`, `library:details-toggle`
(the user's click), `library:unselect`, `library:narrow|wide`, `library:menus`, `library:ctxmenu`;
`devwin:page=books|settings` (switches the two pages), `devwin:view=grid|list` (the one toggle on the Books page, named
*Show as a Grid* / *Show as a List*), `devwin:layout` (prints the dialog height, the parent height and the current page
for `device_fit`), `devwin:notes=on|off`, `devwin:autosync=on|off`, `devwin:syncask` (summary dialog),
`devwin:syncconfirm`, `devwin:send`; `library:dump` also prints `device_button`, `device_banner` (including
`dismiss=true`), `details_auto` (with `auto_sync`) and `close_button_in_header`.

## Wave 7 layout and strings

The exact user-visible copy, checked by `device_window_hig` and the wiki scenarios:

- window/menu: **Devices**; individual row: **Device**; pages: **Books**, **Settings**;
- no device: status page **No Device Connected** — **Connect your e-reader with USB, or add a WebDAV folder below**;
- device list group: **Books on the device**, suffix **Sync Reading State**, a **Select All / Select None** popover, one
  **Show as a Grid** / **Show as a List** toggle, hover/selection checkboxes and a bottom **Remove from Device** with
  **Remove N Books from the Device?** confirmation;
- send group: **Books to send**, empty row **Select books in the library, then choose Send to Device**, button
  **Send 1 Book** / **Send N Books**;
- Settings groups: **Sending options** (Format, Save path, Kobo shelf, *Add my notes to the copy sent to the device*),
  **Sync** (**Always sync when connected**, off by default) and **Cloud folder (WebDAV)**;
- header tooltip: **Connected e-reader: sync, send and manage books**;
- banner: `adw::Banner` **E-reader connected** with **Sync** and its built-in dismiss only.

## Follow-up: Books to send, shared lists, explanations

* `devices::plan_send` / `conversion_of` (pure, in `src/devices.rs`) decide the format, "sent as is" or converted, the notes
  count and the warnings (file missing, too big, already on the device, bad template, no format). The Device window preview
  and `send_full` use the same chain and the same `conversion_of`, so they cannot differ (test
  `the_real_send_does_what_the_plan_said`). An EPUB sent to an EPUB device is now copied as is (it used to be run through the
  "cleaned up" converter) unless a plugin or patch takes part (`plugins::convert_is_hooked`).
* `src/ui/devices_list.rs`: `PickList`, one widget for both lists (rows and tiles). Check box column first, fixed 40x60 cover
  slot, one-line titles, click anywhere toggles, Shift-click range. Hooks: `devwin:click=send|device:<i>[:shift]`,
  `selectall`, `none`, `sendselect=`, `devwin_align:` in the dump (x of the check box and cover slot per row).
* The section appended to a sent book is now titled "My notes" (`send_notes::APPENDIX_TITLE`).
* Explanations live as constants (`device_sync::AUTO_SYNC_*`, `NOTES_*`, `sharing::KOSYNC_*`) and are checked by unit tests.
  Findings while verifying them against the code: KOSync accounts are the reader's own (registered from KOReader, any name
  and password), not the Share password; the KOSync routes skip the Share password, and registration is open; Bookx stores the
  positions only and does not feed them into the library's progress.
* Not covered by a scenario: a real mouse click on a row or tile (the hook calls the same `click` function the gesture calls).
