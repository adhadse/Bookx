# Sync UX notes (internal)

Why the Sync window and Preferences → Sync look the way they do, what was wrong before, and where each setting went.
Not for the wiki; the user-facing text is `wiki/docs/progress-sync.md`, `koreader-sync.md` and `devices-and-sharing.md`.
Code: `src/sync_model.rs` (the information architecture and the state, unit tested), `src/ui/sync_window.rs`,
`src/ui/sync_prefs.rs`, `src/ui/share.rs`, `src/ui/kosync.rs`, `src/annotation_sync_ui.rs`.

## Research applied

Source: Jakob Nielsen, *Progressive Disclosure*, Nielsen Norman Group, 2006
(<https://www.nngroup.com/articles/progressive-disclosure/>). GNOME HIG, *Boxed Lists*
(<https://developer.gnome.org/hig/patterns/containers/boxed-lists.html>), for the row patterns (switch row, entry row,
expander row, one or two controls per row, “rows that open another view end in an arrow”).

The three points of the NN/g article that shaped the design:

1. **Show only a few of the most important options first.** What is on the first display tells the user it is
   important, so the first display holds what almost everybody needs: one status, one main action, the choices. The
   Sync window has a status card with one big button and three choice cards with one button each; Preferences → Sync
   starts with a four-step checklist. Rare and technical settings are not on the first display.
2. **Offer the larger set on request, and make the way there obvious.** Everything else sits behind one clearly
   labelled **Advanced** row (a collapsed expander) whose subtitle names what is inside (server address, port and
   firewall, HTTPS, book matching, privacy, reverse proxy, requirements): strong information scent. Only two levels:
   the first display and Advanced (NN/g: more than two levels get users lost). The same Advanced exists in the Sync
   window (reverse proxy, port and firewall, server activity, button to Preferences → Sync).
3. **Group by the task, not by the technology.** The groups follow what the user is doing (set up Bookx sync; what it
   does; plug in a cable; share books), and the checklist is *staged* disclosure for the one task that has an order
   (server → account → plugin → test on the device), each step saying when it is **Done**. Window and page use the same
   words and the same four steps.

NN/g also says the split must be right: everything frequently needed up front. That is why the server switch, the
account (User name, Password, Register, Log in) and the plugin copy buttons are on the first display of the page, and
why the library server address, which the e-reader needs, is shown under the step-1 switch.

## Stepper (2026-11-20, second round)

Source: Material Design 1, *Steppers* (<https://m1.material.io/components/steppers.html>): “steppers convey progress
through numbered steps”; vertical steppers for narrow layouts with the content under the step; *editable* steps can be
revisited; *non-linear* steppers let the user enter at any step; a connector line joins the circles; active and
inactive circles differ in colour; “don’t embed steppers within steppers or use multiple steppers on one page”;
steppers fit when one field determines the next (server → account → plugin → test).

libadwaita has no stepper, so `src/ui/stepper.rs` is a small reusable widget: a numbered round badge (a check icon when
done), title with a short status, a connector line, the step’s content in a revealer, one step open at a time. It is
**editable and non-linear** (any header opens its step, an open done step stays done, clicking the open one closes it);
with nothing picked the first step that is not done is open and follows the settings (turning the server on opens the
account step). Headers are flat buttons (Tab, Enter, Space), with a stable accessible name (“Step 2 of 4: Create your
sync account, current, alice”) and the expanded state; colours are libadwaita named colours (accent for the open step,
success for done), so light, dark and high contrast follow the theme. The rules are pure functions (`active_index`,
`states`, `connector_visible`, `accessible_name`) with unit tests; the widget only draws them. `compact` puts the status
beside the title for the Sync window card. One stepper per surface, no nesting.

Used for: the Setup checklist of Preferences → Sync and the Wi-Fi card of the Sync window. Audit of the other flows:
Backup (one dialog with a destination list), Text to Speech (independent downloads), MCP (switches and snippets),
Calibre import / convert (one dialog each) have no ordered multi-step setup, so they stay as they are; the stepper is
there for the next flow that has one.

**No information-only rows.** The Sync page has no `AdwActionRow` that only shows text. Explanations are in the group
description, a tooltip, or the body of the step they belong to (plain wrapped text above the controls); live status is
the description of the Advanced group or the step status; feedback (errors, the result of Register) is a label under
the controls. The Requirements table is a small grid inside its expander. `prefs:sync-req` reports `info_rows=` and the
scenario `sync_prefs_ia` asserts 0.

**Highlights, notes and bookmarks is on by default** (`ShareSettings::annotations`, regression test
`annotation_sync_is_on_by_default_and_independent_of_the_server`): nothing listens until the server is started;
with Bookx sync on (step 1) the annotation store exists; with the switch off the server serves none. Existing configs that
never stored the key get the new default; an explicit off is kept.

## What the owner saw (before)

- Two preference pages (“Devices and Sharing”, “Sync”) plus a menu entry “Share Library” for one job. The word
  “KOSync” (an internal protocol name) was in titles, switches and the wiki page title.
- The Share Library window was a settings dump: nine switches and entries (password, port, HTTPS, certificate files)
  before anything said what to do. A newcomer could not tell *what to use when* or *in which order*.
- The account needed by the KOReader plugin was nowhere to be found: it was a field in a different page called
  “Sync reading position”, and nothing said that there is no pre-existing KOReader account.
- “Which password?” was a recurring confusion: the library (OPDS) password, the sync account and a KOReader account.

## Before / after

Before:

```
Main menu: Share Library | Device
Preferences: Devices and Sharing (USB switches) | E-mail | Sync (KOSync client account + switch)
Share Library window: Share switch, LAN, password, port, KOReader progress sync switch, HTTPS (3 rows),
  address, KOReader address, highlights switch, plugin installer, last sync, Sync now
```

After:

```
Main menu: Sync | Device
  Sync window
    status card:  Bookx sync is off / ready / connected   [Set up Bookx sync | Sync now]   Open Preferences → Sync
    card:  Bookx sync with KOReader (Wi-Fi)   Use this when…   ✓/○ server, account, plugin   device menu path   [Start setup | Open setup guide]
    card:  USB device                          Use this when…   connected / not connected         [Open device window]
    card:  Share books (OPDS)                  Use this when…   on / off                            [Start | Stop library server]
    Advanced (collapsed): reverse proxy, port and firewall, server activity, every setting
Preferences → Sync
    Setup checklist   1 server switch + address · 2 account (User name, Password, Register, Log in, result inline)
                      · 3 plugin (Copy to the device, Save to a folder) · 4 test on the device (menu path)
    What Bookx sync does   one plain row + the switch for highlights, notes and bookmarks + last sync + Sync now
    USB device sync        two switches + Device window
    Library servers (OPDS) switch + address
    Advanced (collapsed)   status lines · server address + Use Bookx server · position switch, book matching, device name
                           · network access, library password, port, HTTPS and certificates · reverse proxy guide
                           · request limits · server activity · privacy and offline mode · e-mail · Requirements table
```

## Rules the code follows

- **One settings bus.** Every row that shows a sync setting writes the settings and calls `settings_bus::sync()`;
  every view (page and window) follows it (`share::follow`) and re-reads the settings and the running server. The
  window has no switch that Preferences also has with its own state: its buttons call the same functions
  (`share::set_server`, `share::enable_bookx_sync`). A 3 s timer catches what no setting change announces (a cable, a
  device that synced).
- **Cloud-free and offline-respecting by default.** Everything is off until switched on; the only addresses are this
  computer or one the user types. Offline mode keeps refusing online look-ups and does not stop the user’s own servers
  (shown in Advanced).
- **Step 1 makes the one working path.** “Turn on the Bookx sync server” starts the server, switches Bookx sync on and
  opens it to the local network (an e-reader cannot reach 127.0.0.1); if there is no library password yet it makes a
  random one (12 hex characters from a v4 UUID), shown under Advanced. Turning the library server off stops Bookx
  sync too.
- **Words.** “Bookx sync (extended KOSync)” is said once, on the page, with its explanation; everywhere else “Bookx
  sync”. No `/kosync` anywhere: the only path is `/bookx`. Internal names (`KoSync`, `kosync_client`, `kosync.json`)
  are unchanged.
- **Every setting stays reachable.** Nothing was removed: the old “Devices and Sharing” and “Sync” pages are one page;
  `prefs:pick=Devices and Sharing` and searching that name still find it; the settings that used to be rows are inside
  Advanced (the page search looks into a collapsed Advanced row, row by row).

## Where each old setting went

| Old place | New place |
|---|---|
| Preferences → Devices and Sharing: sync automatically, add my notes | Preferences → Sync → USB device sync |
| Preferences → Sync: server address, user name, password, Register, Log in | Setup checklist step 2 (server address under Advanced, filled in from the running server) |
| Preferences → Sync: Sync the reading position, Match books by, This device is called | Advanced |
| Share Library: Share this library | Library servers (OPDS); the window’s “Share books” card |
| Share Library: KOReader progress sync, address | Setup checklist step 1 |
| Share Library: Allow other devices, password, port, HTTPS, certificate, key, new certificate | Advanced |
| Share Library: highlights and notes switch, last sync, Sync now | What Bookx sync does |
| Share Library: plugin installer | Setup checklist step 3 |
| Preferences → Privacy | unchanged; linked from Advanced |
| Preferences → E-mail | unchanged; linked from Advanced |

## Tests

- Unit: `sync_model` (states off / ready / connected, the checklist, the three cards, words), `sync_menu_tests` in
  `library.rs`.
- Broadway (`build-aux/ui-tests-sync.sh`): `sync_menu`, `sync_prefs_ia`, `sync_prefs_live`,
  `koreader_annotations_sync_states`, `sync_account_ui`, `sync_account_needs_server`; older scenarios updated
  (`prefs_sidebar`, `prefs_search`, `sharing_kosync`).
- Stepper: `sync_stepper`, `sync_notes_default` (Broadway), unit tests in `src/ui/stepper.rs` and `sync_model`.
- Real session (`build-aux/session/scenarios_sync.py`): hamburger → Sync, light/dark, 900x600 and 1280x800, English
  and German, with screenshots.

## Caveats

- “Connected” means a KOReader device has sent something to this computer (annotation sync); the reading-position
  endpoint does not tell the UI who called. A device that only syncs its position shows “ready”.
- The plugin step is “done” when Bookx copied the plugin (a remembered flag); a hand-copied plugin is not detected.
- At 900x600 the status card and the Wi-Fi card fit on the first screen; the two other cards are one scroll away.
- Not tried on real hardware (no Kobo or KOReader device was used; the plugin copy goes to a fake device folder).

## Server and login did not survive a restart (RCA)

- **Cause 1:** only `share.kosync` ("answer Bookx sync") was saved; whether the server *ran* lived in
  `Sharing` (memory) and nothing started it at launch. Step 1's switch shows `running && share.kosync`,
  so after every restart it read Off.
- **Fix:** `ShareSettings.running` (saved by `ui::share::set_server` through `ShareSettings::remember`:
  off is always remembered, on only after a successful start, so a busy port keeps the wish);
  `ui::share::restore` runs from `app.rs` at launch when `restore_at_launch()` (running and settings valid).
  A failed restore shows its error on the page and keeps the wish. `Config::save` now writes through a
  temp file and rename. No stale whole-`share` write-back was found (all other writers clone-modify one
  field or use `..c.share.clone()`).
- **Cause 2:** the account row of step 2 was a fixed "Log in" row with a fixed hint; the result of
  Register/Log in was only shown until the page closed. Fix: `KoSyncPrefs.logged_in` (set on a successful
  Register/Log in, cleared when server, user or password is edited) and `sync_model::account_status`
  ("Logged in as X", "Account saved: X", "Password needed for X" with a keyring explanation when the
  keyring is locked or missing).
- **Tests:** unit tests in `sharing.rs`, `config.rs`, `sync_model.rs`; Broadway `sync_restart` (seeded
  settings file as a previous run left it).
- **Needs the real setup:** that the real keyring is unlocked at login (the password comes from it).
