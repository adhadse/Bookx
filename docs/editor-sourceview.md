# The editor on GtkSourceView (internal notes)

Date: 2026-11-12. Owner decision: `docs/enhancements.md` item 5. Supersedes the "No, not now" of
`docs/editor-ui-design.md` section 4. Public documentation: `wiki/docs/editing.md`.

## What changed

The code view is a `sourceview5::View` with `sourceview5::Buffer`s (crate `sourceview5` 0.11, feature
`v5_6`; the GNOME 49 runtime has libgtksourceview 5.18, the dev machine 5.22; the README names 5.6 as the
minimum because `Snippet::new_parsed` needs it).

| Piece | File | Notes |
|---|---|---|
| widget setup, language and scheme choice, hidden characters | `src/ui/editor/codeview.rs` | pure decisions (`language_id`, `pick_scheme`, `hidden_chars`, `search_text_for_context`, `find_status`) are unit tested; the rest builds widgets |
| marks column | `ui/editor/gutter.rs` (renderer), `src/editor_marks.rs` (data) | a `GutterRenderer` subclass; `editor_marks::Marks::build` combines changed lines, findings and misspelled counts |
| completion | `ui/editor/completion.rs` (provider), `src/editor_complete.rs` (logic) | one `CompletionProvider` subclass; `proposals()` is the same code the dev hook `complete=` runs |
| snippets | `src/editor_snippets.rs`, `ui/editor/snippets.rs` (dialog), expansion in `completion.rs` | GtkSourceView snippet syntax; JSON in the config folder |
| live pass | `ui/editor/live.rs` | debounced 400 ms: spelling, marks, completion data; tag pair at the caret; search glue; view options |
| spelling | `src/editor_spell.rs` | `bookx_editing::spell` with the book's dictionary, personal list `spelling-words.txt`, session ignore list |
| view settings | `src/editor_prefs.rs` | minimap and hidden characters, `editor.json` (the main config is untouched) |
| change bar engine | `bookx_editing::compare::changed_lines` | Added / Modified / Removed lines of the new text; Patience diff |
| tag pair engine | `bookx_editing::tags::highlight_pair` | |
| old highlighter | `src/ui/syntax.rs` | kept: fallback when GtkSourceView has no language for a file (it also still defines the `syn-*` tags and the `marked-region` tag) |

The old `install_gutter` drawing area, the shared tag table and the `Lang` driven highlighting of every
buffer are gone. `Lang::for_path` is still used for "Reformat" and the preview.

## Search: who does what

* `sourceview5::SearchContext` (one per buffer, one shared `SearchSettings`, wrap-around on): the
  highlight of every match in the open file and the match count / position shown in the find bar
  ("n of m in this file"). Only the open file's context highlights.
* `src/editor_search.rs` (fancy-regex): navigation (Find next/previous across files), Replace, Replace
  All, Find All, the scopes (this file, all text, styles, marked text), `\1` / `$1` replacements and
  look-around.
* Why not only SearchContext: it knows one buffer, uses GLib's PCRE flavour (no `\1` in the
  replacement from our dialog, different dot-all handling: the `(?s)` flag is put in front when the
  dot-all toggle is on), and cannot express the marked-text scope.
* When the context cannot be used (the "Marked text" scope, or `regex_error()` is set) the status label
  falls back to the engine's count (`update_find_count`).

## Gutter details

* Changed lines: the baseline is the newest checkpoint (restored once per checkpoint id and cached in
  `Editor::baseline`), or, when there is no checkpoint, the text of the file when the editor first showed
  it (`Editor::original`).
* Findings: `run_check` stores each finding with a `TextMark`, so it follows edits. They are not re-run
  while typing; the user presses F7 again.
* Misspelled counts come from the same pass that underlines.
* Hover text: `Marks::tooltip`.

## Caveats / not done

* Squiggles, marks and completion data are refreshed 400 ms after the last change or scroll; a very
  fast typist sees stale underlines for that long.
* Spelling: only the book's language (not per-element `lang`), only XHTML/HTML files, only files up to
  700 kB, only the window plus 40 lines.
* Completion: prefix matching only; no fuzzy matching; no anchor text previews (CE-346), no
  insert-image/link dialogs (CE-354). Accepting a proposal is GtkSourceView's default (Enter, or click).
* Snippets: no `${2*}` selected-text placeholder (not in GtkSourceView); the Tab handling is a capture
  phase key controller on the view that checks the word before the caret and an "inside the last
  expanded snippet" region (two `TextMark`s), because GtkSourceView offers no query for an active
  snippet. A word in prose that equals a trigger (`p`, `h`, `link`…) expands on Tab.
* The marks renderer is drawn with the icon theme's symbolic icons; a missing theme shows nothing.
* Not tested by a scenario: mirrored snippet placeholders, `<style>` blocks inside XHTML being
  highlighted as CSS, the Tab key itself (the scenarios call `expand_at_caret`; there is no synthetic
  key event path in Broadway), right-click popup (the menu builder is tested, the gesture is not),
  clicking the minimap.
* Fixed during development: completion `populate` used the iterators of the `CompletionContext`,
  which go stale when the text changes while the popup is being prepared (an endless loop in
  `gtk_text_buffer_get_text` when switching files with an expanded snippet pending). The provider now
  builds a fresh iterator from the buffer's insert mark.

## Tests

* Rust: `editor_complete` (6), `editor_snippets` (4), `editor_prefs` (1), `editor_marks` (3),
  `editor_spell` (4), `ui::editor::codeview` (6), `bookx-editing` `compare::changed_lines` and
  `tags::highlight_pair`.
* UI scenarios (`build-aux/ui-tests.sh`): `editor_view`, `editor_search_context`, `editor_live_marks`,
  `editor_live_spell`, `editor_live_pairs`, `editor_live_hidden`, `editor_live_complete`,
  `editor_live_snippets`, plus all the older `editor_*` scenarios on the new widget.
* Visual check (by hand, Broadway + a browser): the minimap, change bars, error icon, misspelled counts,
  squiggles, tag pair highlight, match highlight and the hidden-character dots were looked at in
  screenshots (the wiki screenshots `editor.png` and `editor-find.png` are from that).
