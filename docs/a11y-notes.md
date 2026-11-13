# Accessibility, command palette, Preferences search, RTL: notes (internal)

Work log and decisions for PP-G-19, PP-G-21, PP-C-1 (palette, searchable Preferences) and PP-C-23. Public text is
`wiki/docs/accessibility.md`; plan for vertical text is `docs/vertical-writing-plan.md`.

## What was built

| Area | Where | Tests |
|---|---|---|
| Page text for screen readers | `src/ui/page_area.rs` (`PageArea`, a `gtk::DrawingArea` subclass implementing `GtkAccessibleText`), `visible_page_text` and `refresh_a11y` in `src/ui/canvas.rs`, pure part `src/a11y.rs` (`PageText`, granularities, selection and offset mapping) | `a11y::tests` (9), `a11y_page_text`, `a11y_orca` (session), `page_text_for_readers` (Broadway) |
| Control names + audit | `src/ui/audit.rs` (`label()`, `controls()`, `report()`), rule `a11y::control_problem`; labels added to scales, switches, drop-downs, text views, back buttons of the reader popovers, editor view/minimap/find scope/contents entries | `a11y_audit_library`, `a11y_audit_reader`, `a11y_audit_windows` (Broadway, fail on any new unnamed control), `a11y_names` (session, real AT-SPI tree) |
| Keyboard-only reading | reader key handler: Shift+arrows, Ctrl+Shift+arrows, Shift+Up/Down select by character / word / sentence (`a11y::move_focus`, `BookCanvas::key_select`), Enter commits (`commit_selection`), `d w t h n Ctrl+C Esc` act on the selection; Ctrl+Enter saves a note; the canvas takes the focus when the reader is shown; `.book-canvas:focus-visible` outline in `style.css` | `a11y::tests`, `keyboard_selection` (Broadway), `a11y_keyboard_only` (session: no mouse from palette to saved note) |
| Command palette | `src/palette.rs` (registry `COMMANDS`, `fuzzy`, `rank`, entries), `src/ui/palette.rs` (AdwDialog), `reader.*` action group in `reader.rs`, `App::show_palette` in `app.rs`; Ctrl+K and Ctrl+Shift+P | `palette::tests` (12), `palette_library`, `palette_reader` (registry check: every command has its action), `a11y_palette_keys` |
| Preferences | `src/ui/preferences.rs`: four pages (General, Reading, Devices and Sharing, Extensions), `set_search_enabled(true)`, `index()` collects every row (page, section, title, subtitle) for the palette, `show_at()` opens on a row | `prefs_search` (Broadway), `a11y_prefs_search` (session, the dialog's real search) |
| RTL | `Package::rtl` (`page-progression-direction`), `OpenBook::is_rtl`, `gestures::right_is_forward` / `mirror_tap`, canvas `set_rtl` (taps, swipes), reader keys | `opf::rtl_tests`, `gestures::rtl_tests`, `rtl_book`, `ltr_book_tap` (Broadway), `a11y_rtl_keys` (session) |
| High contrast | `a11y::high_contrast_palette` used by `apply_prefs` when libadwaita says `is_high_contrast()` and the colours are the theme's; re-applied on change | `a11y::tests` (every reader theme has text contrast >= 7:1, links and muted text >= 3:1), `a11y_high_contrast` |
| Harness | AT-SPI registry failure detection and one retry (`harness.py`, `selftest.py`, `session-tests.sh`), `GDB=1` | see `docs/test-notes.md` |

## Decisions

* **A subclass of `DrawingArea`, not hidden child labels.** One accessible with the Text interface lets Orca navigate by line,
  word and sentence, keeps selection and caret in one place and costs no extra widgets. The text is the visible page only
  (paragraph per canvas item), refreshed from the draw handler when the page, scroll position, layout or selection changed
  (`a11y_key`), in an idle callback so no AT-SPI traffic happens inside `draw`.
* **Selection is by unit, there is no free caret in the canvas.** Shift+arrows move the free end of a selection starting at
  the first character of the page. A screen reader's caret navigation works on the page text (`set_caret_position`), but it does
  not move the canvas selection.
* **Palette = gio actions.** Commands are `app.*`, `lib.*`, `reader.*` action names so the palette runs exactly what menus run;
  `palette_reader` fails if one is missing. Reader actions were new (the reader had only buttons and keys).
* **Preferences search.** libadwaita's `AdwPreferencesDialog` already has a search across pages; it matches row titles and
  subtitles. We split the one page into four so results are grouped, renamed the e-mail rows ("Mail server port" ...) so they are
  findable, and put the section titles into the palette index. Own filtering was not built because it would duplicate libadwaita.
* **RTL.** Layout mirroring of the chrome is GTK's job (it follows the locale). The page direction of the *book* is the spine's.
  Mirrored: arrow keys, tap zones, swipes. Not mirrored: the footer arrow buttons and the slider (they keep "previous on the left"),
  the sidebar side.

## Bugs found on the way

* Orca crashed the app: GTK walks the attribute arrays as NULL-terminated, gtk4-rs gives NULL arrays for empty lists. Found only
  by running Orca (`a11y_orca`); the python AT-SPI client never calls `GetAttributeRun`.
* A book opened from the palette left the keyboard focus on the Back button: Enter went back to the library. The reader now grabs
  the focus for the canvas when it is mapped.
* `Node.get_text()` in the harness silently returned `""` (it called the method on the wrong object): the old "canvas exposes no text"
  note hid it.
* Shift+Right used to turn the page; it now selects (page turning is Right, Page Down, Space).
* The first version of `a11y_keyboard_only` pressed Escape twice and left the book: Escape clears a selection, and leaves the book when
  there is none (existing behaviour, documented in the wiki).

## Honest limits

* No long listening test with Orca; the Orca scenario checks that Orca finds the canvas, the app survives, Page Down works and Orca
  reads a line when its caret moves. Orca's browse mode takes the arrow keys, so Right/Left do not reach the app there.
* Images have no alternative text in the page text; links inside the page cannot be focused; fixed-layout books and comics have no text.
* GTK popover menu items have no names in the AT-SPI tree (known quirk, see `docs/test-notes.md`); the Broadway audit does not check
  `GtkPopoverMenu` contents (their labels come from the menu model).
* The audit's `describe()` can only know names set with text, a tooltip or `ui::audit::label()`; widgets named otherwise
  (`labelled-by` relations set by libadwaita) are treated as opaque. The session audit reads GTK's own computed names, so it is the oracle.
* Reduced motion: GTK honours `gtk-enable-animations` for its own transitions (revealers, stacks, CSS transitions). Bookx draws no
  animation of its own. In the sandbox the setting did not reach the app, so it was not checked end to end; `a11y-env:` prints what the app sees.
* `GtkAccessibleText::set_caret_position` is installed by hand (unsafe, only on GTK >= 4.22) because gtk4-rs without the `v4_22` feature
  cannot override it; `set_selection` keeps GTK's default (refuses). Enabling `v4_22` would raise the build requirement above the Flatpak
  runtime (GNOME 49, GTK 4.20).
* The palette is not in the editor window; the editor's own palette design (`docs/editor-ui-design.md` section 6) is still open.
* High contrast was checked with the gsettings key in the sandbox only; the system theme (HighContrast) is libadwaita's.
* RTL books: the reader's footer buttons and the sidebar are not mirrored; vertical writing is a plan only.

## Counts (final run)

See the final report of the change; scenario and test counts are in the output of `build-aux/ui-tests.sh`, `build-aux/session-tests.sh`
and `cargo test`.
