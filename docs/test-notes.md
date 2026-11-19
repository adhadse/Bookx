# Test notes (internal)

## Real-session tests (`build-aux/session-tests.sh`)

Why: the Broadway suite (`build-aux/ui-tests.sh`) was green while the dictionary and Wikipedia buttons did not work on the
owner's GNOME/Wayland desktop (bug 1 in `docs/bugs.md`), and other regressions were missed too. Broadway has no real
compositor: popovers are drawn inside the window, there is no `xdg_popup`, focus or grab handling, and the dev hooks
(`reader:ui-dict-btn`, `reader:ui-dblclick`...) call handlers directly, so GTK's gesture and popup machinery is never exercised.

### Options evaluated

| Option | Verdict |
|---|---|
| `gnome-shell --headless --wayland --no-x11 --virtual-monitor 1280x800` inside `dbus-run-session` | **Chosen.** The real shell: real Mutter, real xdg_popup handling, hot corners, the Activities overview, `org.gnome.Mutter.RemoteDesktop`, `org.gnome.Shell.Screenshot`, AT-SPI bridge. Starts in about 3 s, runs fine without a GPU session or seat. |
| `mutter --headless --wayland --virtual-monitor` | Works too, but has no top bar, no overview, no Screenshot service and no portals. It is not what the owner runs. Kept as a fallback idea only. |
| ydotool / libei / dogtail / wtype | Not installed, and not needed: Mutter's RemoteDesktop D-Bus API injects pointer/keyboard events at the compositor level (the same path as a real device). |
| ScreenCast stream for absolute pointer motion | Needs PipeWire. Not needed: relative motion plus a corner clamp gives exact absolute positions (see below). |

### Design

* `session-tests.sh` runs every scenario in its **own** `dbus-run-session` (private bus config that only auto-starts the
  services GNOME Shell and the app need), its own `XDG_RUNTIME_DIR`, `HOME`, config, cache and data folders,
  `GSETTINGS_BACKEND=keyfile` (animations, hot corners, screen lock off), its own gnome-shell, library
  (`bookx-cli library add` of the test book) and dictionary. `HOME` is the sandbox, so nothing touches the real library or
  config; `BOOKX_DEVICE_ROOTS` points at an empty folder (otherwise the app lists e-readers that are mounted on this
  machine: the first run found the owner's real Kobo and would have offered to sync with it). The harness refuses to start when
  `BOOKX_LIBRARY`, `HOME` or the config folder is not inside the sandbox.
* `build-aux/session/sandbox.py`: starts gnome-shell, starts `at-spi2-registryd` by hand (the systemd user unit does not exist
  here), owns the bus name `org.gnome.SettingsDaemon.MediaKeys` (gnome-shell only lets that name and the portal call
  `Screenshot`), creates a RemoteDesktop session and injects input with `NotifyPointerMotionRelative`, `NotifyPointerButton`,
  `NotifyPointerAxisDiscrete`, `NotifyKeyboardKeysym`. Absolute pointer positioning: clamp into the bottom-right corner, then move
  relatively (the top-left corner is the shell's hot corner: a fresh session also starts in the Activities overview, which the
  harness leaves with Escape).
* `build-aux/session/atspi.py`: the AT-SPI tree of the app (`Atspi` from PyGObject). Used to find widgets by role and name, to
  read labels, `sensitive` and `showing` states, and to get click positions. **Quirk:** GTK4 on Wayland reports window-relative
  coordinates, 7 px left and 5 px above what is drawn (plus the 32 px top bar), so clicks use the origin `(7, 37)`. GTK4 popover
  menu items have no accessible names: their text is read with tesseract OCR of a screenshot crop (tesseract is optional, the
  OCR checks are skipped without it). The page canvas exposes no text either, so word positions come from screenshot pixels
  (`glyph_near` finds a letter to double-click).
* Screenshots (with the pointer) are saved per step in `<target>/session-results/<scenario>-<step>.png`; look at them when a
  scenario fails. `KEEP=1` also keeps the compositor, app and AT-SPI logs; `WAYLAND_DEBUG=client` records the Wayland protocol.
* Limit: every session uses about 15 inotify instances (GNOME Shell, ibus, portals, the bus). The limit is 128 **per user**
  and shared with the real desktop, so the script computes how many are free and never starts more sessions than fit
  (`JOBS_FORCE=1` overrides). The first attempt with 8 sessions at once exhausted them ("Too many open files" in dbus-broker).

### Scenarios (all in `build-aux/session/scenarios.py`)

`library_open_book`, `reader_page_turn` (keys, the Next button, the wheel), `lookup_dictionary`, `lookup_wikipedia` (network),
`lookup_translate` (network), `highlight_and_note` (colour, popover stays, note, saved note, remove), `theme_change` (Dark, Sepia,
Light by pixel), `library_context_menu`, `header_menus` (library and reader menus, search, text appearance, contents),
`news_dialog`, `editor_edit_and_replace` (opens the editor, types, Ctrl+H, Replace All), `second_book`.
Vertical writing (`build-aux/session/scenarios_vertical.py`, see `docs/vertical-writing-plan.md`): `vertical_reading` (a generated Japanese book: AT-SPI character boxes prove columns run down and leftwards, real Left/Right page turns of a right-to-left book, a real drag across two columns with the exact selection, the highlight painted as tall bands, a search match behind turned Latin letters) and `vertical_aozora` (a real novel from Aozora Bunko; skips without the cached file). AT-SPI key events carry ASCII only, so the search term is Latin.

### Findings of the first runs (the owner's open bug 1, and two more)

1. **Reproduced the owner's symptom before any fix**: double-click a word, the action popover appears (good), click the
   dictionary or Wikipedia button: the button gets its hover style, nothing happens, no page appears. Then the same for the
   highlight colours (so those never worked either on a real compositor). Instrumenting the handlers showed that the buttons'
   `clicked` was never emitted while press and release did arrive at the popover.
   **Root cause:** `reader.rs` put a `GestureClick` on the popover content that called `set_state(Claimed)` on every press ("shield":
   keeps presses on the popover from reaching the page canvas, which clears the selection). A claim by that parent gesture
   cancels the buttons' own click gesture. Broadway's dev hooks bypass gestures, so the old scenarios stayed green.
   **Fix:** an `EventControllerLegacy` in the bubble phase of the popover that returns `Propagation::Stop` for button, touch and
   scroll events (after the buttons had them). Pure part: `popover_event_stops_here` (unit test).
2. After choosing a highlight colour the popover vanished at once (its row grows by the Remove button: a popover resized in
   place disappears on a real compositor; the same cause the earlier look-up fix worked around). Fix: `represent_popover`.
3. About one look-up in two killed the app: `Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display` and, from
   `WAYLAND_DEBUG=client`, `xdg_wm_base#44.error(4, "wl_surface@52 already has a buffer committed")`. GTK4 re-uses the popover's
   surface and a popdown immediately followed by popup (`show_lookup_page`, the note buttons) races with the first commit.
   Fix: `repopup` / `repopup_then` close the popover, change it, and open it again 80 ms later (`REPOPUP_PAUSE_MS`).
   Checked with the loop in "Flakiness" below.

### Honest limits (what Broadway still does better or worse)

* Broadway is faster (about 1 s per scenario vs 15-40 s), needs no GNOME, can run 25 scenarios at once, and the dev hooks reach
  state that no widget exposes (canonical text, hyphenation, TTS mocks, device mocks). These stay in `ui-tests.sh`.
* The headless shell has no GPU session or fractional scaling (scale 1), no real clipboard manager, no real portals dialogs
  (file chooser, open-URI are not driven), no touch screen input (touch paths such as long press are not covered), only one
  1280x800 monitor, light Adwaita with the system fonts of the machine running it. It is not the owner's exact desktop:
  extensions, Wayland scale, other compositors (KDE, Sway) and X11 are not covered.
* Pixel and OCR checks depend on fonts and theme; they use wide tolerances (pixel colour classes, substring after OCR).
* Real-network scenarios (`lookup_wikipedia`, `lookup_translate`) need internet (`NET=0` skips them); they accept "a clear error"
  as a pass when the service is down, but a real answer is what is expected.
* The AT-SPI coordinate origin `(7, 37)` was measured, not derived: if GTK changes it, every click scenario fails together.

### Flakiness

Final state: 12 scenarios, 3 full runs in a row, all green (about 95 s with 2 sessions at once). History: before the `repopup`
fix the look-up scenarios failed in about 3 of 6 runs (Wayland protocol error 71); with 8 sessions at once, sessions failed to
start (inotify exhaustion, see above). Another agent's `pkill bookx` (or `./run.sh`) kills the app under test: the harness
recognises an app that died from SIGTERM/SIGKILL (`RETRY`, exit code 75) and runs that scenario once more. Run the suite three times
in a row after changing the harness. The Broadway suite (`ui-tests.sh`) was not re-run for this change.

### The AT-SPI registry failure (retry)

The harness starts `at-spi2-registryd` by hand (no systemd user unit). Once a run failed because GTK logged
`Could not activate remote peer 'org.a11y.atspi.Registry'` and no window ever showed up in the accessibility tree.
`harness.py` now scans `app.log`, `shell.log` and `atspi-registry.log` (`registry_failure_in`, a pure function with
`build-aux/session/selftest.py`, which `session-tests.sh` runs first) when a scenario cannot find the app window or crashes. A match
exits `run_scenario.py` with code 76 (`REGISTRY_FAILURE`) and `session-tests.sh` runs the scenario once more in a fresh session
and work folder, like exit 75 (app killed from outside). Only one retry: a second failure is reported. The retry path was checked with
`RETRY_MARKER=/tmp/opencode/m ONLY=_retry_probe build-aux/session-tests.sh` (first attempt fakes the log line, the second passes).
The real failure was not reproduced again, so the patterns are the one message seen plus two likely variants.

### Accessibility scenarios (`build-aux/session/scenarios_a11y.py`)

`a11y_page_text` (the canvas text through the AT-SPI Text interface: words, lines, sentences, caret, page turns),
`a11y_keyboard_only` (open a book with Ctrl+K, page, select, look up, highlight, note, notes tab, focus ring: no mouse),
`a11y_palette_keys`, `a11y_prefs_search`, `a11y_rtl_keys`, `a11y_names` (every interactive node in the real tree has a name,
library, menus, reader, popovers, palette), `a11y_high_contrast` (gsettings `high-contrast=true` makes the page black on
white), `a11y_orca` (the real Orca, skipped without it). `GDB=1 ONLY=... build-aux/session-tests.sh` runs the app under gdb.

Orca found a crash the AT-SPI python client could not: GTK's `GetAttributeRun` / `GetAttributeValue` walk the attribute arrays as
NULL-terminated, and gtk4-rs hands GTK NULL arrays when the Rust implementation returns an empty list. `PageArea` now always
returns one attribute. `a11y_orca` is the regression test. Same lesson for text: `GetText` expects a C string.

Sandbox limit: the app reports `animations=true` for `enable-animations=false` in the keyfile (the portal path GTK reads in
this private bus does not see it), so reduced motion could not be checked end to end; the app reads GTK's setting.


### Reader UX scenarios (`build-aux/session/scenarios_reader_ux.py`, `build-aux/ui-tests-reader-ux.sh`)

* `reader_sidebar_toggle` (bug 9): a generated big book (2020 contents entries, 500 highlights through `reader:bulk-highlights`), ten
  real openings and closings (toggle button and F9), visibility by AT-SPI, frame timing printed by the app (`reader:side-watch`).
  Numbers from the first run in the headless GNOME Shell (debug build): first frame 0-20 ms, frame work at most 3.4 ms. Broadway cannot
  measure this (its frame clock waits hundreds of milliseconds by itself), so the Broadway scenario `side_perf` only budgets the work
  inside a frame and checks that nothing hidden is built.
* `reader_selection_handles`: the balls are found by pixel colour (accent blue 54,133,227, which the popover's lighter blue swatch is not),
  dragged with the real pointer; the selection comes back through the AT-SPI Text interface of the page. No touch input exists in the
  harness, so the finger path (word snapping, `GestureDrag` from a touch device) is only covered by the pure geometry tests and the
  `dev_drag` hook.
* `reader_footer_popover`, `lookup_wikipedia_image` (canned Wikipedia through `BOOKX_WEB_MOCK`), `lookup_wikipedia_image_net` (real Wikipedia).
* Findings of this work: F9 never closed the sidebar and Escape left the book instead of closing it (found by the 10x toggle test).

### Selection popover over dialogs (`src/ui/overlay_guard.rs`)

Root cause: the reader's selection action popover has `autohide` off (a click on the page must extend or clear the selection, not
just close it). Such a popover is a surface of its own and nothing else closes it, so on a real compositor it stayed above
AdwDialogs (Preferences, Properties, the book map, shortcuts) and other menus. Fix, in one place: the guard closes it when
`AdwApplicationWindow:visible-dialog` becomes set, when a window is added (`GtkWindow::toplevels()` grows), when the window stops
being active, and when any popover on a menu button of the reader (and the footer popover) is shown. It also makes a tooltip give way.
The selection stays; the popover is not brought back by itself (a new selection, or Enter, shows it). Scenarios:
`selection_popover_dialogs` (Preferences, Properties, book map, shortcuts, footer popover; fails without the guard, 6 of 13 checks),
real session `reader_selection_popover_under_dialog` (screenshot `prefs-over-selection`). The TTS and speed-reading cards are
overlay children of the page, so they are below dialogs by construction. Hook: `dialog:state|close`, `reader:properties`.

### Code-editing helpers with real input (`build-aux/session/scenarios_editor_code.py`)

Real pointer and keys in the editor of the Alice test book (public domain, `~/.cache/bookx-testbooks`): `editor_code_ctrl_click`
(Ctrl+click on a link, on a class and on a selector, then F12), `editor_code_context_menu` (right-click: the *Go to Definition*
entry follows the spelling entries), `editor_code_format_keys` (Ctrl+B / Ctrl+I on a keyboard selection, `</` closes the tag and
typing the rest of the tag is not doubled), `editor_code_complete_keys` (file popup and Tab, anchor popup with text snippets),
`editor_code_insert_dialogs` (Ctrl+K, filter, Down, Enter; Ctrl+Alt+M), `editor_code_dialogs_small` (the dialog fits 900x600),
`editor_code_rename_class_dialog` (right-click, *Rename Class*, new name, Enter, the style sheet follows), `editor_code_marks_visual`
(colour swatch and red squiggle by pixel count), `editor_code_tag_keys` (Ctrl+<, Ctrl+>, Ctrl+Alt+>).

* A character's screen position comes from AT-SPI `Atspi.Text.get_character_extents` on the code view (window coordinates plus the
  usual `(7, 37)` origin): it is exact enough to Ctrl+click a single class name. A character that is scrolled out horizontally is
  not on screen, so the scenarios type the test line at the top of the page.
* Typing `>` after an open tag adds the closing tag (Preferences option): scenarios that want to type a tag by hand delete the
  added tag or leave the caret where the editor put it.
* Findings: the context menu entries of a second right-click gesture were wiped by the spelling handler (`remove_all`), so the code
  entries are appended from that handler; the *Wrap in Tag* prompt did not take the keyboard focus (fixed); `</` completion
  doubled the closing tag when it was typed in full (fixed with the type-over of what was inserted).
