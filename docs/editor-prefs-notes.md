# Editor preferences, shortcuts, snippets, search leftovers (wave 5, agent `ed-prefs`)

Rows: CE-086, 107, 110, 357, 383, 385, 386, 387, 400-406, 424, 425 (status in `calibre-editor-gap-analysis.md`).

## Where things are

| Piece | Files |
|---|---|
| Settings (`EditorLook`, `editor-look.json`), CSS colour literals, auto-close tag decision | `src/editor_prefs.rs` |
| Editor page of Preferences, live apply, CSS colour tags, auto-close typing, append-final-output | `src/ui/editor/prefs_page.rs` |
| Editor shortcut registry (id, title, default keys, palette target) | `src/editor_keymap.rs` (group *Editor* in `src/shortcuts.rs`) |
| id → `Input`, controllers rebuilt on every change, palette key display | `src/ui/editor/keymap.rs` |
| Snippet engine (selected text `${N*}`, flat expansion, candidates, JSON, templates) | `src/editor_snippets.rs` |
| Insert Snippet list, Tab in the search fields | `src/ui/editor/snippets_ui.rs` |
| Manage Snippets (Test box, import/export, templates) | `src/ui/editor/snippets.rs` |
| Fuzzy search | `src/editor_search_fuzzy.rs`, `Mode::Fuzzy` in `editor_search_modes.rs` |
| `functions`, persistent data, `append_after_marked` | `src/editor_search_fn.rs` |
| Ctrl+F in the preview | `src/ui/editor/preview_find.rs` |

## Decisions and caveats

* The look settings are in their own file because the other editor settings (`editor.json`) are saved by several windows from stale
  copies; a shared file would lose the new fields.
* A change in the page is applied to the window through the settings bus; the toggles that other menus own (wrap, minimap,
  whitespace) are flipped through their actions so the menus keep their state.
* Shortcuts: Editor keys only overlap with Editor keys (`Group::overlaps`), the window has its own controllers. Menu items do not show
  the key text (they are plain `GMenu` entries); the palette and Preferences do. The keys of the file list (F2, Delete) and of the code
  view itself (undo, clipboard, Tab) are not registry entries.
* Cursor width and blink (CE-357): GTK 4 has them only as display-wide settings; not implemented.
* Preview: "lines above the sync point" and per-family fonts (CE-402) are not offered.
* Selected-text stops use GtkSourceView's own snippet engine: the body is rewritten (`with_selection`) before `push_snippet`, escaping
  `$`, `}` and `\` of the selection. Every starred stop of a snippet takes the selection.
* Bug fixed on the way (found with real keys): the end mark of the snippet region had right gravity, so text typed right after an
  expanded snippet extended the region and Tab no longer expanded the next trigger there when text was selected. The end mark now
  stays put (regression: real-session `editor_snippet_tab`).
* `functions`: a called function gets no `functions` itself (one level); persistent `data` is per library folder and function name,
  stored hashed in `search-function-data/` (no user text in file names).
* Security (docs/security.md §8): the snippet import reads a user-chosen JSON file (serde, each snippet validated by `problem`, template
  labels cannot hold `/`, `\` or `.`); style schemes are loaded from the user's own config folder by GtkSourceView; no network,
  no process, no new dependency.

## Not verified

* Nothing was tried on hardware other than the sandboxed GNOME session of the test harness (Wayland, mutter) and Broadway.
* The marker of marked regions in the *scrollbar*: the minimap shows them, the scrollbar itself does not.
