# Shortcut research (enhancement 21)

Internal notes: which keys other readers use, what Bookx took over, and what it left alone. Public list:
`wiki/docs/shortcuts.md`. The registry is `src/shortcuts.rs` + `src/palette.rs` (`COMMANDS`).

## What the other readers do

Sources: Calibre viewer manual, "Keyboard shortcuts" table (https://manual.calibre-ebook.com/viewer.html, read on
2026-11-13, so this column is checked). Foliate, KOReader and Okular are from their shipped shortcut help and from
memory of using them; entries marked (?) were not re-checked against the current release.

| Action | Calibre viewer | Foliate | KOReader | Okular |
|---|---|---|---|---|
| Search | Ctrl+F, `/` | Ctrl+F | (menu; touch) | Ctrl+F |
| Next / previous match | F3 / Shift+F3 | Ctrl+G / Ctrl+Shift+G | – | F3 / Shift+F3 |
| Go to page / location | Ctrl+G | Ctrl+L (location) (?) | (menu) | Ctrl+G |
| Contents | Ctrl+T | F9 sidebar | (menu) | F7 sidebar |
| Bookmark: toggle / list | Ctrl+Alt+B new, Ctrl+B list | Ctrl+D | (top-right tap) | Ctrl+B add |
| Highlights panel | Ctrl+H | in sidebar | (menu) | – |
| Larger / smaller / reset text | Ctrl+Plus / Ctrl+Minus / Ctrl+0 | Ctrl+Plus / Ctrl+Minus / Ctrl+0 | (pinch, menu) | Ctrl+Plus / Ctrl+Minus |
| Zoom with the wheel | Ctrl+wheel | Ctrl+wheel | pinch | Ctrl+wheel |
| Fullscreen | F11, Ctrl+Shift+F | F11 | – | Ctrl+Shift+F |
| Next page | Space, PageDown, arrows | Space, PageDown, arrows | tap right, page-turn keys | Space, PageDown |
| Previous page | Shift+Space, PageUp | Shift+Space, PageUp | tap left | Shift+Space, PageUp |
| Start / end of the book | Ctrl+Home / Ctrl+End | Home / End (?) | – | Home / End |
| Back / forward in history | Alt+Left / Alt+Right | Alt+Left / Alt+Right | "Go back to previous location" (menu) | Alt+Left / Alt+Right |
| Read aloud | Ctrl+S | – | (menu) | Ctrl+Shift+S (speak) |
| Layout: paged / flow | Ctrl+M | (preferences) | – | – |
| Book info / metadata | Ctrl+N, Ctrl+E | Ctrl+I (properties) (?) | (menu) | Ctrl+I (?) |
| Edit this book | Ctrl+D | – | – | – |
| Preferences | Ctrl+Comma | Ctrl+Comma | – | Ctrl+Shift+Comma |
| Open a book | (library) | Ctrl+O | – | Ctrl+O |
| Close the book / window | Ctrl+Q quits | Ctrl+W | – | Ctrl+W |
| Quit | Ctrl+Q | Ctrl+Q | – | Ctrl+Q |
| Help | – | F1 | – | F1 |
| Book map (page browser) | – | – | menu "Book map" | – |

Common core (three or more of the readers): Ctrl+F, F11, Ctrl+Plus/Minus/0, Ctrl+G, Alt+Left/Right, Space and
Shift+Space, PageUp/PageDown, Ctrl+Q, Ctrl+W (close), Ctrl+O (open), F1 (help).

## What Bookx did

Reader keys that already existed and stay: arrows, PageUp/Down, Space, Shift+Space, Home/End, Ctrl+Left/Right (chapter),
Ctrl+F, Ctrl+B (toggle bookmark), Ctrl+G, Alt+Left/Right, Ctrl+Plus/Minus, F11, Ctrl+R (reload), Escape (back to the
library). The page keys are now listed in the registry as defaults too (they were hard-coded before), so the Keyboard
Shortcuts page, the palette and the wiki show them.

New defaults (all can be changed; `shortcuts.json` always wins):

| Action | New key | Taken from |
|---|---|---|
| Contents (sidebar) | F9, Ctrl+T | Foliate, Calibre |
| Bookmarks and highlights list | Ctrl+Shift+B, Ctrl+H | Calibre (Ctrl+H), the list sits next to the bookmark key |
| Reset text size | Ctrl+0 | Calibre, Foliate |
| Larger text | also Ctrl+= (same key as Ctrl+Plus on most layouts) | all |
| Book map | M | KOReader's book map |
| Next theme (Light, Sepia, Dark, Black) | T | – (with text selected, T still translates) |
| Read aloud (play / pause) | R | Calibre has Ctrl+S, which is taken by Save in the editor; R is free in the reader |
| Open another book | Ctrl+O (reader) | Foliate, Okular |
| Book properties | Ctrl+I (reader and library) | Okular, Foliate (?) |
| Back to the library | Ctrl+W (also Escape) | Foliate, Okular |
| Edit book (library) | Ctrl+Shift+E | Ctrl+E is edit metadata (Calibre uses Ctrl+E for the book info; kept as it was) |
| Online help | F1 | Foliate, Okular, GNOME convention |

Not taken, and why:

* **F3 / Shift+F3 and Ctrl+G / Ctrl+Shift+G for next / previous match.** The reader's search lives in the sidebar
  and has no "next match" action yet. Ctrl+G stays *Go to…* (Calibre and Okular use it that way).
* **Ctrl+L (lock).** There is no lock feature; Ctrl+L is *Go to line* in the book editor.
* **Ctrl+D.** In the library it is *Add to Favourites*; in the reader the bookmark is Ctrl+B. Foliate's Ctrl+D (add
  bookmark) would be a second key for the same thing and is left out so each key has one meaning per window.
* **Ctrl+S (Calibre's read aloud).** Save in the editor.
* **Ctrl+M (Calibre's paged / flow).** Layout is under *Layout* in the menu; M alone is the book map (KOReader).

### Reassignments

No default of an existing action was taken away. Keys that already had a meaning in the reader window were kept; the
new defaults went to free keys. The only keys with two meanings by context are T (translate when text is selected,
otherwise next theme) and the selection keys D, W, H, N that only work while text is selected.

### How a key reaches an action

1. The user's `shortcuts.json` (customised actions) is looked up first (`handle_reader_key`).
2. The reader's built-in keys (page turns, selection, Ctrl+B, F11 …) run next, so mirrored books and selection mode
   keep working.
3. Everything else is looked up in the registry's reader defaults (`Shortcuts::default_reader_action`), which runs the
   action. A default key of an action the user rebound does nothing (it is "disabled"), as before.

Keys are ignored while a text field has the focus.

Tests: `shortcuts::tests::the_well_known_reader_keys_are_defaults`, `no_two_default_actions_share_a_key_in_the_same_window`,
`keys_without_built_in_handling_find_their_reader_action`, `the_users_choice_wins_over_the_new_defaults`; UI scenario
`reader_new_keys`; real-session scenario `new_reader_keys`.
