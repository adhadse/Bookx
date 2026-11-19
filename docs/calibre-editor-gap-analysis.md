# Calibre "Edit book" gap analysis for the Bookx editor

Research document (no code changes). Date: 2026-11-12. Roadmap rows covered: E1-E25 (`docs/bookx-feature-roadmap.md`, Part 3; the owner's decisions in Part 2 were read first: Rust only, wasm plugins, Rhai for scripting after Wave 2, no GTK in logic crates).

**Sources read**

* Calibre manual: `edit.html` (all sections), `function_mode.html`, `snippets.html`, `diff.html`, `regexp.html`, `regexp_quick_reference.html`, `generated/en/ebook-edit.html`, and the API page named `polish.html` (it documents the container/polish API, not a user-facing "Polish books" page).
* The manual does **not** contain the shortcut table, the exact names of the search-panel options, the check-book error classes, the report columns or the preferences pages. These were taken from the Calibre source on GitHub (`src/calibre/gui2/tweak_book/{ui,boss,search,file_list,reports,spell,check,preferences,editor/*}.py` and `src/calibre/ebooks/oeb/polish/check/*.py`, master). Calibre's checks have **class names, not numeric ids**; this document uses the class name as the "error id".
* Not readable / not found: the video tour, the screenshots (images were not viewed), the MobileRead forum function examples, the W3C accessibility spec, a manual page for "Polish books" in the main window (not part of the editor), "search placeholders" and a separate "Tag Search" mode (neither exists in the manual or in the current source: search modes are `normal`, `fuzzy`, `regex`, `function`; "Search ignoring HTML markup" is the separate text-search panel). "Downgrade EPUB 3 to 2" does not exist in Calibre either.

**Bookx code read:** `src/ui/editor.rs`, `src/editor_tools.rs`, `src/ui/syntax.rs`, `crates/bookx-editing/src/*` (all modules), `crates/bookx-core/src/{check,format,toc,opf,book}.rs`, plus `bookx-convert` (smarten, remove unused CSS) and `bookx-sources` (cover generation) where a feature lives outside `bookx-editing`.

**Status legend**

* **DONE** = usable in the editor UI today.
* **ENGINE ONLY** = implemented and tested in a crate, no editor UI (or the engine exists but only as a trait/mock without a real backend).
* **PARTIAL** = some of it works in the UI; the "What is missing" column says what is not there.
* **MISSING** = nothing exists (building blocks that could be reused are named in the design note).

Effort: S (< 1 day), M (1-3 days), L (about a week), XL (more than a week or new subsystem). Priority: P1 core editing flow, P2 common, P3 niche.

## 1. Summary

| Status | Count |
|---|---|
| DONE | 254 |
| PARTIAL | 4 |
| MISSING | 2 |
| NOT PLANNED | 5 |
| **Total rows** | **265** |

(Counted from the row statuses on 2026-11-20; the priority and "top 15 gaps" lists below are the research-time record.)

By priority:

| Priority | DONE | PARTIAL | ENGINE ONLY | MISSING |
|---|---|---|---|---|
| P1 | 36 | 18 | 3 | 7 |
| P2 | 38 | 19 | 12 | 42 |
| P3 | 16 | 7 | 12 | 55 |

Top 15 P1 gaps (ordered by value for the core editing flow):

1. CE-023 rename via `markup::rename_paths` (current rename can corrupt other references) + CE-007 checkpoints for non-`run_op` changes.
2. CE-066/070 regex dialect (look-around, `\1`) and mode dropdown; CE-072/073 scopes; CE-084 saved searches.
3. CE-026 multi-select file list, enabling CE-027 merge many, CE-034 delete, CE-024 bulk rename.
4. CE-011 tabs and CE-010 quick open.
5. CE-243/244 preview <-> editor position sync (needs source offsets in the document model).
6. CE-334/335/352/351 tag matching, jump, insert/remove/split tag, rename block tag.
7. CE-345/346/354 auto-complete for file names and anchors, insert link/image.
8. CE-342 Ctrl+click to open files / jump to class rules.
9. CE-080 go to line.
10. CE-270..275 interactive spell checking (ignore, change all, add to dictionary, per-language dictionaries).
11. CE-160/161 check-book help and per-error auto-fix (plus skip/next/previous, CE-162/163).
12. CE-224 side-by-side diff view for checkpoints (engine ready).
13. CE-120/121/124/126 ToC: navigate on select, generation options, drag and drop, pick a location in the preview.
14. CE-248/249 split at cursor and split dialog (headings by level, class, page breaks).
15. CE-136 Fix HTML per file with an HTML5 parser fallback; CE-039 new blank file / templates; CE-029 drag and drop spine reorder.

Headline findings (verified in code when the analysis was written; items 3 and 4 are fixed since, see the row statuses of CE-007 and CE-023):

1. The editor shell is small (since 2026-11-12 the code view is a GtkSourceView `View`, see `docs/editor-sourceview.md`): one code view that swaps buffers, a file list, a ToC tab, a find bar, a results list and a Tools menu (`src/ui/editor.rs`). Most of Calibre's *depth* (tabs, completion, snippets, tag matching, live CSS, saved searches, function mode) is absent.
2. `bookx-editing` already contains engines for many tools the UI does not expose: filter styles, CSS transforms, semantics, embed fonts, remove unused fonts, external-link check, download external resources, broken-link repair, split by ids/page breaks, multi-file merge, text/archive diff, spelling replace/ignore/user words, word/char/CSS/link/image reports. Wiring these is the cheapest big win (Wave 2).
3. **Correctness issue in the UI:** `Editor::rename_file` (editor.rs) rewrites references with a plain `text.replace(from, to)` over every text file, which also rewrites other names that merely contain the old name (for example renaming `a.png` rewrites inside `data.png`) and ignores `url()` and fragments. `bookx_editing::markup::rename_paths` is the tested, reference-aware implementation and should replace it (CE-023). Needs a regression test.
4. `Editor::run_op` snapshots before every tool (good), but Find/Replace All, ToC edits (Generate overwrites the whole ToC), Delete File, Move File and Add Files do **not** create a checkpoint, although Calibre does for global operations.
5. `check_all` (F7) merges `core::check::check_book` with the 87-rule validator (`validate::RULES`, epubveri-style ids), which already covers most of Calibre's OPF/link/duplicate-id classes, but the list has no help text, no per-error auto-fix, no skip-type, no next/previous error and no accessibility toggle (section 6).
6. The regex engine is Rust `regex`: no look-around, no back-references in the pattern, replacement uses `$1` not `\1`. Calibre users (and the regexp manual pages) rely on look-around, `\1`, `\K`, recursion, `(?s)`, atomic groups. A different engine (for example `fancy-regex` for the pattern, with a `\1` to `${1}` translation for the replacement) is a prerequisite for function mode and saved searches that are portable (CE-066).

## 2. Gap tables by manual section

Row format: `ID | Calibre feature (name in the manual, one-line behaviour) | Bookx status | Where in Bookx | Missing / design note | Effort | Priority`.

### 2.1 Basic workflow, windows, files of the editor

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-001 | Open a book with Edit book (right-click book, Edit book) | DONE | `Editor::init` opens `lib.open_book(book_id)`; opened from the library | none | - | P1 |
| CE-002 | File list on the left, double-click opens a file for editing (HTML, CSS, image) | DONE | `rebuild_files`, `select_file` (`editor.rs`) | none (images open in a viewer page, HTML/CSS/XML in the code view) | - | P1 |
| CE-004 | Save (Ctrl+S) writes the changes into the e-book | DONE | `Editor::save` (flush, write ToC, `replace_archive`) | none | - | P1 |
| CE-005 | Save a copy / Save a copy and edit it here (Ctrl+Alt+S, Ctrl+Alt+Shift+S, in source) | DONE | Book menu (header): Save a Copy… (Ctrl+Alt+S), Save a Copy and Edit It… (Ctrl+Alt+Shift+S) (`window_features.rs`, `save_copy`; the edit variant adds the copy to the library and continues with it) | none; scenario `editor_x_window` | S | P2 |
| CE-006 | Unsaved-changes prompt on close | DONE | `Input::CloseRequested` (`adw::AlertDialog`) | none | - | P1 |
| CE-007 | Checkpoints are also created automatically for every automated tool and global search/replace | DONE | `run_op` (every Tools entry, Add/Delete/Rename/Move/Merge/Bulk rename/New file/Link stylesheets) and `create_checkpoint` for Replace All, Contents Generate and metadata changes | none: every whole-book change first takes a checkpoint named “Before: …” (scenario `editor_checkpoints`) | S | P1 |
| CE-008 | Revert to before / Revert to after (global undo/redo, Ctrl+Left / Ctrl+Right) | DONE | Book menu: Revert to Before the Last Change (Ctrl+Alt+Z) / Revert to After (Ctrl+Alt+Shift+Z) (`editor_state::undo_target`, `redo_still_valid`); each revert leaves a redo point, a new change starts a new history | Ctrl+Left/Right were not used: they move the caret by word | M | P2 |
| CE-009 | Edit next / previous file in the spine (Ctrl+Alt+Down / Up) | DONE | Ctrl+Alt+Down / Ctrl+Alt+Up (`nav.rs`, `step_file`) | none | S | P2 |
| CE-010 | Quick open a file by typing part of its name (Ctrl+T) | DONE | Ctrl+P popover (`nav.rs`, `files::quick_open`: prefix > substring > scattered letters, name before path) | Calibre’s shortcut is Ctrl+T | S | P1 |
| CE-011 | Tabs for several files at once, close tab / close others / close to the right (Ctrl+W, Ctrl+Alt+W, Ctrl+Shift+W) | DONE | Tabs; Ctrl+W close, Ctrl+Shift+W close other tabs, Alt+Shift+W close tabs to the right, also in the tab context menu (`editor_state::tabs_to_close`) | Calibre’s Ctrl+Alt+W is taken by Wrap Selection in Tag, so the shortcuts differ | L | P1 |
| CE-012 | Toggle code line wrapping in all editors | DONE | Book menu: Wrap Long Lines (remembered in `editor-window.json`) | none | S | P2 |
| CE-013 | Open another book / open an unzipped folder as a book / new book (Ctrl+O, File menu) | DONE | Book menu: Open Another Book… (library list, Ctrl+O), Open an Unzipped Folder…, New Book… (`editor_tools::archive_from_folder`); the window switches to the book, asking before it drops unsaved changes | none | M | P3 |
| CE-014 | Import an HTML or DOCX file as a new book (File menu) | DONE | Book menu: Import HTML, DOCX or Text as a New Book… (any format the library imports) | none | M | P3 |
| CE-015 | Restore state of previously edited book (open tabs, positions) | DONE | Open tabs, the selected tab and the caret of each file are stored per book (`editor-state.json`) and restored; switch in the Book menu | Scroll position is not kept; can be switched off | M | P3 |
| CE-016 | Window layout: dockable panels, nested docks, toolbars chosen by the user, toolbar icon size | MISSING | fixed `adw::OverlaySplitView` | **not planned**: dockable panels do not fit an adaptive libadwaita window (and are untestable headless); the layout stays fixed | - | P3 |
| CE-017 | User manual (F1) | DONE | F1 and Book menu: Help on Editing opens https://bookx.adhadse.com/editing/ (`gtk::UriLauncher`) | none | S | P3 |
| CE-018 | Command line `ebook-edit [opts] [path] [name ...]` with `--select-text`, `--detach` | DONE | `bookx --edit FILE [name…] [--select-text TEXT]` and `bookx-cli edit …` (adds the file to the library if needed, opens the editor, opens the named files, selects the text) (`editor_state::parse_edit_args`) | Unit tested, and the scenario `editor_x_edit_cli` goes through the same entry point (dev hook `edit-file:` builds the same request as the command line). `--detach` is not needed: the editor is a window of the app | M | P3 |
| CE-019 | Editor plugins (Tool class: `create_action`, `register_shortcut`; Boss API: `edit_file`, `show_editor`, `add_savepoint`, `rewind_savepoint`, `save_book`, ...) | DONE | ABI v2 (additive) in `bookx-plugin`: kind `editor-action`, `[[action]]` (label, group, suggested shortcut), permissions `editor.read` / `editor.write` shown in Preferences, one host import `editor_call` with `list_files`, `read_file`, `write_file`, `add_file`, `rename_file` (references follow), `current_file`, `get_selection`, `replace_selection`, `insert_at_cursor`, `get_metadata`, `set_metadata`, `add_checkpoint`, `show_message`; limits per action; the host adapter `src/ui/editor/plugin_host.rs` runs the action on a copy of the book inside `run_op` (one checkpoint "Before: <label>", undoable, nothing changes when it fails); Tools → Plugins and the command palette; example `examples/plugins/trim-whitespace` (Rust, wasm32) | `crates/bookx-plugin/tests/editor_plugin.rs` (permission denial, escape attempts, limits, runaway loop, checkpoint order, real Rust example), `editor.rs`/`manifest.rs` unit tests, `plugin_host` unit tests, scenarios `editor_plugin_trim`, `editor_plugin_off`. Not done: running an action on the selection with GTK undo grouping (selection edits are applied to the file text and undone with the checkpoint), a plugin store, plugin-defined dialogs, keyboard shortcuts are only suggested | XL | P3 |

### 2.2 The File browser

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-020 | Files grouped by category: text, styles, images, fonts, misc; text files in reading order, others alphabetical | DONE | Files are grouped Text / Styles / Images / Fonts / Package / Other; the text group is in reading order, the others alphabetical (`editor_tools::browser_order`) | none | S | P1 |
| CE-021 | Hover shows size and the full path | DONE | Hover shows path, size, media type and the notes of the file (`editor_tools::file_tooltip`) | The path is still not in the status bar (the editor’s status line shows the open file) | S | P2 |
| CE-022 | Icons marking special files (cover image, titlepage, OPF = metadata, NCX = ToC) and tooltips for "not in manifest", "non-linear", "not in spine", "in spine twice", "invalid font" | DONE | Icons with tooltips for the package document, NCX, navigation document, cover, title page, not in manifest, not in spine, twice in spine, not in the main reading order (`book_tools::file_notes`) | No “invalid font” mark | M | P2 |
| CE-023 | Rename a file (Rename, F2 in source) and update all links | DONE | `Editor::rename_file` -> `editor_tools::rename_one` -> `bookx_editing::files::rename_files` over `markup::rename_paths`; F2 and the ⋯ menu | regression tests: `a.png` leaves `data.png` alone, `url()` in CSS, fragments `ch.xhtml#id`; moving to another folder is not offered (name only) | M | P1 |
| CE-024 | Bulk rename: prefix + start number, "rename by book order" option | DONE | ⋯ menu, Bulk Rename… (`files::bulk_rename_plan`, reading order, zero padded) | none | M | P2 |
| CE-025 | Change the file extension of all selected files | DONE | Files menu: Change Extension… (references follow) | none | S | P3 |
| CE-026 | Multi-select in the file browser (Shift/Ctrl) | DONE | file list is `SelectionMode::Multiple`; a single selection opens the file | none | M | P1 |
| CE-027 | Merge selected files (Ctrl+M); choose master file; HTML or CSS only; links migrated | DONE | ⋯ menu, Merge Selected… (master chosen in the dialog; `split_merge::merge_files`) | XHTML only; merging CSS is not in the engine; no Ctrl+M | M | P1 |
| CE-028 | Merge by dragging files onto another text file | DONE | DONE: drop a page on the middle of another page row (the row is tinted; the top and bottom quarters still reorder): a confirmation dialog, then `merge_files` on a checkpoint (the dragged page goes to the end of the target, links repaired); `struct_hub.rs`, `files_extra.rs`. Tests: `editor_struct_merge_drag` (Broadway), `editor_file_drag_merge` (real session, real drag) | only pages (not stylesheets or images) | M | P3 |
| CE-029 | Reorder the spine by drag and drop between rows | DONE | Drag a page row onto another: the insertion line shows above or below, drop reorders the reading order (`book_tools::drop_index`, `Editor::drop_row`); up/down buttons stay | Real pointer drag: session scenario `editor_file_drag_reorder`; Broadway scenarios call the drop handler (`x:drop`). No keyboard shortcut for moving | M | P1 |
| CE-030 | Mark non-linear spine items / drag to top to make linear | DONE | Files menu: Toggle Not in Main Reading Order (`linear="no"`), shown with an icon | Dragging to the top does not make a file linear again; use the toggle | S | P3 |
| CE-031 | Mark as cover image (images) | DONE | `Input::SetCover` ("Use as Cover" button on image page, `OpenBook::set_cover`) | button only on the image page; also add to the file row context menu | S | P2 |
| CE-032 | Mark as cover page / titlepage (HTML) | DONE | Files menu: Use as Cover Page / Use as Title Page (guide and landmarks) | The page is not moved in the spine | S | P2 |
| CE-033 | Mark as Table of Contents (nav document, EPUB 3) | DONE | Files menu: Use as Table of Contents; warns when the page has no `nav epub:type="toc"` | none | S | P3 |
| CE-034 | Delete selected files (context menu or Delete key); OPF references removed, other references left for Check book | DONE | Delete key / ⋯ menu / `files::delete_files`; multi-select; checkpoint; the summary counts links that now point nowhere | none | S | P1 |
| CE-035 | Export a file out of the book (Export) | DONE | ⋯ menu, Export Selected… (save dialog for one file, folder for several) | none | S | P2 |
| CE-036 | Replace a file with another file; links updated if the name changes | DONE | Files menu: Replace with File… (name field; links follow when the name changes) | none | M | P2 |
| CE-037 | Export all selected files (choose a folder) | DONE | same entry, folder dialog, `files::export_files` numbers clashes | none | S | P3 |
| CE-038 | Copy selected files to another editor instance / paste from other editor instance | MISSING | not planned: Calibre’s copy and paste between editor instances needs a shared clipboard format; Bookx exports and adds files (Export Selected…, Add Files…) instead | - | M | P3 |
| CE-039 | New file (images/fonts/HTML/CSS...) dialog: import a resource or create a blank HTML/CSS by name; option to auto-link stylesheets into new HTML; templates for new files | DONE | New File… lists the user’s templates (`.xhtml`, `.html`, `.css` in the `editor-templates` folder of the configuration; `{title}` is replaced) | none | M | P1 |
| CE-040 | Import files into book (several at once) | DONE | `Input::AddFiles` (multiple selection) | none | - | P2 |
| CE-041 | Link stylesheets: pick CSS files (drag to order), optionally remove existing links, insert `<link>` into all selected HTML files | DONE | ⋯ menu, Link Stylesheets… (`files::link_stylesheets`; checked list in the given order, optional removal of existing links, idempotent) | no drag-to-reorder of the stylesheet list | M | P2 |
| CE-042 | Open a file with an external program ("Open with…") | DONE | Files menu: Open With… writes the file to a temporary folder, asks for the program, and offers to update the book when that program saves it | The real launcher is a portal dialog; tests set `BOOKX_OPEN_WITH_PROGRAM` (an absolute path, started without a shell) and the scenario `editor_x_open_with` checks the round trip | M | P3 |
| CE-043 | Select all / de-select all files of a category | DONE | Files menu: Select → All Text Files / Stylesheets / Images / Fonts / Select None | none | S | P3 |
| CE-044 | Show full file paths in the file browser (preference) | DONE | Files menu: Show Full Paths (remembered) | Flat list with the path in the label, not a folder tree | M | P3 |
| CE-045 | Compress images losslessly (also a Tools entry) | DONE | `Tool::CompressImages` -> `images::optimize_images` | no per-image selection; results listed in results panel | - | P2 |
| CE-046 | Browse images in the book visually (grid) | DONE | Files menu: Browse Images… (thumbnail grid with size and “unused”, open or delete the selection) | none | M | P3 |

### 2.3 Search & replace

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-060 | Search panel (Search, Find/replace, Ctrl+F) with Find and Replace boxes | DONE | `find_rev` revealer, `find_entry`, `replace_entry`; Ctrl+F / Ctrl+H | none | - | P1 |
| CE-061 | Find next / previous (F3, Ctrl+G / Shift+F3, Shift+Ctrl+G), wraps around | DONE | DONE: F3/Shift+F3, Ctrl+G/Shift+Ctrl+G, wraps and says "wrapped around"; scenario `editor_search_bar`, session `editor_search_bar_session` | add Ctrl+G / Shift+Ctrl+G aliases; "Wrap" and "Direction" are fixed (always wrap) | S | P2 |
| CE-062 | Replace current match (Ctrl+R) | DONE | `replace_one` (replaces if selection equals a match, then finds next) | button only; add shortcut | S | P2 |
| CE-063 | Replace and find next / previous (Ctrl+], Ctrl+[) | DONE | DONE: Ctrl+], Ctrl+[ (replace and find next/previous), Ctrl+R; menu "Replace & Find Previous" | add "replace and find previous" | S | P3 |
| CE-064 | Replace all (Ctrl+A in panel) over the chosen scope | DONE | `replace_all` takes the checkpoint “Before: Replace All” | no per-file summary dialog | M | P1 |
| CE-065 | Count all (Ctrl+N) over the chosen scope | DONE | DONE: Count button, "N matches in M files" in the bar, per-file tooltip | add a "Count" that reports total per scope and per file | S | P2 |
| CE-066 | Regular expression mode with the full regex engine (look-ahead/behind, back-references, `\K`, atomic/branch-reset groups, recursion, `(?s)`/`(?m)`, named groups `\g<name>`, `\1` in replace) | DONE | DONE: fancy-regex, `\1`/`$1`/`\g<name>` | Rust `regex` lacks look-around, back-references and `\K`. Needed: `fancy-regex` (or `pcre2` if allowed) and translation of `\1`/`\g<name>` to the replacement; document the dialect in the panel's info popover | L | P1 |
| CE-067 | "Dot matches all" option (shown in regex/function modes) | DONE | DONE: dot-all button shown in Regex/Function modes | checkbox in the options popover | S | P2 |
| CE-068 | Case sensitive option | DONE | `case_btn` ("Aa") | default in Calibre is case-insensitive too | - | P1 |
| CE-069 | Whole word option | DONE | DONE: W toggle (`\b…\b`) | wrap the pattern in `\b…\b` for Normal mode | S | P3 |
| CE-070 | Search mode drop-down: Normal / Regex / Function (Fuzzy in source) | DONE | DONE: mode drop-down Text/Regex/Function/Tag (+ "ignore markup" toggle); fuzzy not done | replace the toggle with an `adw::ToggleGroup`/dropdown: Normal, Regex, Function, (Fuzzy optional, P3) | S | P1 |
| CE-071 | Scope: Current file | DONE | `scope` dropdown "This file" | none | - | P1 |
| CE-072 | Scope: All text files (HTML) | DONE | DONE: Text files / All files | Calibre separates HTML from style files; rename the existing option "All files" and add proper "Text files" | S | P1 |
| CE-073 | Scope: All style files | DONE | DONE: Style files | filter on `.css` | S | P1 |
| CE-074 | Scope: All files | DONE | DONE | covers text-based files only (fine) | S | P2 |
| CE-075 | Scope: Selected files (in the file browser) | DONE | DONE: Selected files (needs multi-select in the file list) | needs multi-select (CE-026) | S | P2 |
| CE-076 | Scope: Open files (tabs) | DONE | DONE: Open files (tabs); scenario `editor_search_bar` | needs tabs (CE-011) | S | P3 |
| CE-077 | Scope: Marked text (Mark selected text, Ctrl+Shift+M) - search only inside a marked region, multiple regions | DONE | Ctrl+Shift+M marks the selection (`TextMark`s); scope “Marked text”; scenario `editor_goto_mark` | none | M | P3 |
| CE-078 | Highlight all matches in the editor while searching | DONE | DONE: sourceview SearchContext highlight; own tag highlight for tag/ignore-markup modes (capped at 3000) | `TextTag` "match" applied by a debounced scan of the current buffer | M | P2 |
| CE-079 | Search history in the Find/Replace boxes | DONE | DONE: history popover on both boxes, last 20, `search-history.json` | store last 20 in config; show in a popover | S | P3 |
| CE-080 | Go to line (Search, Go to line, Ctrl+.) | DONE | Ctrl+L popover, `editor_search::parse_line`; scenario `editor_goto_mark` | none | S | P1 |
| CE-081 | Special characters inside search fields (Insert special character works there; snippets work without placeholders) | DONE | search bar popover (regex escapes) and the full Insert Special Character dialog, which inserts into the focused find / replace field (CE-300) | none | S | P3 |
| CE-082 | Find results list (Bookx addition; Calibre shows matches only through function-mode output) | DONE | `find_all`, results panel (max 500) | raise or page the 500 cap | - | P2 |
| CE-083 | Search ignoring HTML tags (Search ignoring HTML markup panel: find `on a word` across `<i>` tags; replace, find next/previous) | DONE | DONE: `bookx-editing::search_ignore`; rule documented in the wiki; replace keeps inner tags | build a text-only projection with offset map (`markup::text_runs`, `decode_with_offsets`) and map hits back to byte ranges; replacement keeps inner tags (Calibre replaces text then re-flows tags) | L | P2 |
| CE-084 | Saved searches: save current search from the Find box context menu; dialog lists searches (name, find, replace, mode, scope), add/edit/delete/import/export, run several selected searches in one operation | DONE | DONE: Saved Searches dialog, JSON import/export, ordered runs with one checkpoint; `saved-searches.json` | `Vec<SavedSearch>` in config (JSON), `adw::Dialog` with a list; run order = list order; each run = one checkpoint | L | P1 |
| CE-085 | Search in tag attributes ("Tag Search" in the task text) | DONE | DONE: Tag mode, `img[alt=""]`, `a[href^=http]`, replace `attr=value` / `-attr` / `=part`; `bookx-editing::tag_search` | not found as a separate Calibre feature; attribute search works through Regex. Possible Bookx design: "Find in attribute…" using `markup::tag_attrs` | M | P3 |
| CE-086 | Search in preview panel (find next / previous in preview) | DONE | DONE (also CE-247): the preview header has a find bar (next / previous, matches highlighted in the canvas); Ctrl+F with the focus in the preview opens it instead of the code view's bar (`preview_find.rs`) | none | M | P3 |

### 2.4 Function mode (`function_mode.html`)

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-100 | Function mode: a user function `replace(match, number, file_name, metadata, dictionaries, data, functions)` called for every match instead of a replacement template | DONE | DONE (Rhai, `src/editor_search_fn.rs`): `fn replace(m, number, file, meta, data)`, sandbox limits | per roadmap: **Rhai**, not Python. Define `fn replace(m, ctx)`; sandbox, op-count limit, no I/O | XL | P2 |
| CE-101 | `match` argument (regex match object, `.group(n)`) | DONE | DONE | expose `m.group(n)`, `m.start`, `m.end`, named groups | M | P2 |
| CE-102 | `number` argument (1-based counter across a Replace All) | DONE | DONE | counter in the run context | S | P2 |
| CE-103 | `file_name` argument (canonical path inside the book, empty for marked text) | DONE | DONE (empty for marked text) | pass `String` | S | P2 |
| CE-104 | `metadata` argument (title, authors, language ...) | DONE | DONE | map to a read-only Rhai object | S | P2 |
| CE-105 | `dictionaries.recognized(word)` | DONE | DONE: `dict_recognized(word)` (book language dictionary; false without dictionary) | register as a Rhai function | S | P3 |
| CE-106 | `data` dict persisting during one Replace All | DONE | DONE | Rhai `Map` in the context | S | P2 |
| CE-107 | `functions` access to other saved functions and persisted data | DONE | DONE: `replace` may take a sixth parameter `functions`: `functions["name"].invoke(m, number, file, meta, data)` runs another saved function (shared `data`; a called function cannot call further ones); `// @persistent_data` keeps `data` per library in `search-function-data/` between runs | none | M | P3 |
| CE-108 | `replace.file_order = 'spine' / 'spine-reverse'` | DONE | DONE: `// @file_order spine/spine-reverse` | per-function attribute `file_order` | S | P3 |
| CE-109 | `replace.call_after_last_match` (extra call with `match = None`) | DONE | DONE: `// @call_after_last_match`, final output in the results window with Copy | flag in function metadata comment | S | P3 |
| CE-110 | `append_final_output_to_marked`, `suppress_result_dialog` | DONE | DONE: `// @suppress_result_dialog` and `// @append_final_output_to_marked` (the final output is inserted on a new line after the last marked range of the open file and marked too; `append_after_marked`) | none | S | P3 |
| CE-111 | `print()` debugging shown in a "Debug output" window after the run | DONE | DONE: Debug output in the Function results window | output pane in the function editor dialog | S | P2 |
| CE-112 | Built-in functions (Title-case text ignoring tags, Upper/Lower case, Capitalize, Swap case, smarten hyphens...), Create/edit function dialog | DONE | DONE: Functions dialog + editor (SourceView, Test on Sample, line-numbered errors, import/export) | ship a handful of Rhai builtins; `GtkSourceView`-like editor box with Test | L | P2 |
| CE-113 | Example recipes (title-case headings, em-dash hyphens, de-hyphenate with dictionary, auto-number sections, generate ToC from headings) | DONE | DONE: five worked examples ship as built-ins, unit-tested, in the wiki | ship as sample functions and tests | M | P3 |

### 2.5 Automated tools (Tools menu)

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-120 | Tools, Table of Contents, Edit Table of Contents (dialog with the current ToC, edit text on double-click, re-arrange by drag/drop or buttons) | DONE | DONE: sidebar tab "Contents": drag the grip of an entry onto another (top quarter before, bottom quarter after, middle = last child; subtrees move whole), Alt+Up/Down/Left/Right for move, promote, demote, buttons, delete keeps the children, Add at the cursor's nearest `id` with its heading text, titles editable in place, selecting navigates (CE-126), a button points an entry at a place clicked in the preview (CE-124). Pure operations in `bookx_editing::toc_ops` (move, promote, demote, drop, delete, insert, `nearest_anchor`, `normalize`), written through `toc::write_toc` (nav + NCX). Tests: 8 unit tests, `editor_struct_toc` (Broadway: drop handler and ops through dev hooks), `editor_toc_drag` (real session: a real pointer drag onto the middle and the top edge of a row, real Alt+Down and Alt+Right with the focus kept) | no undo for a single move (a checkpoint covers Generate, not each move) | M | P1 |
| CE-121 | Generate ToC from headings (XPath levels such as `h1`, `h2`, `h3`, prefer title) | DONE | Generate… dialog: levels 1-6, replace or add (`toc_gen::generate`) | no class filter and no preview of the tree before replacing (the checkpoint “Before: Generate Contents” can be restored) | M | P1 |
| CE-122 | Generate ToC from links (existing inline contents page) | DONE | Generate… dialog, “From the links of a page” (`toc_gen::links_of`) | none | M | P2 |
| CE-123 | Generate ToC from files (one entry per spine file, title from `<title>`/first heading) | DONE | Generate… dialog, “One entry per file” | none | S | P2 |
| CE-124 | Change the location an entry points to by clicking in a mini preview (green line) | DONE | Contents toolbar button + pick mode of the preview (green line); adds an `id` when the element has none (`preview_map::ensure_id`) | none (the main preview is used, not a separate mini preview) | L | P1 |
| CE-125 | Remove / add ToC entries; a book without ToC gets auto-generate options | DONE | `TocOp::Delete/Add/Generate` | none | - | P1 |
| CE-126 | Table of Contents view (View, Table of Contents): double-click an entry opens its location; right-click edit, refresh, expand/collapse all | DONE | selecting an entry (or Enter/double-click) opens the file and moves the cursor to the `#fragment` (`toc_gen::anchor_offset`) | no right-click menu, no expand/collapse all | S | P1 |
| CE-127 | Insert inline Table of Contents (replaces a previously inserted one) | DONE | `Tool::AddInlineToc/RemoveInlineToc` -> `inline_toc::insert_inline_toc/remove_inline_toc` | options (title, position, depth, stylesheet) exist in `InlineTocOptions` but UI uses defaults; add a small dialog | S | P2 |
| CE-129 | Add cover: choose an existing image or import a new one, EPUB HTML wrapper generated, old cover replaced, OPF marked | DONE | DONE: Tools → Media and Fonts → Set Cover: an image of the book, a file from the computer, or a cover drawn from the title and author (`bookx_sources::cover::generate_cover`); crop to 2:3 / 3:4, size limit, live preview; makes or updates the cover page, marks the image (`cover-image`, `<meta name="cover">`), writes guide + landmarks, deletes the old image when unused, Remove Cover; images are decoded with limits (40 MB, 64 megapixels). `bookx_editing::cover` (9 unit tests), `cover_dialog.rs`; tests: `editor_struct_cover`, `editor_struct_dialogs`, real-session screenshots | no "no svg" / "keep aspect" options (the page is one `img` with CSS `max-height:100vh`) | M | P2 |
| CE-130 | Embed referenced fonts (finds fonts referenced in CSS, searches the computer, embeds) | DONE | Tools → Media and Fonts → Embed Referenced Fonts… (searches the installed fonts, licence flag from OS/2 fsType, restricted fonts cannot be ticked, optional subsetting) | The installed fonts are searched on a worker thread (progress shown); font files over 12 MB are skipped | M | P2 |
| CE-131 | Subset embedded fonts | DONE | `Tool::SubsetFonts` -> `fonts::subset_embedded_fonts` | add "do this last" confirmation (text added later may miss glyphs) | S | P2 |
| CE-132 | Smarten punctuation (dashes, ellipses, quotes) | DONE | Tools → Text → Smarten Punctuation… (quotes, dashes, ellipses, language rules for double quotes, straighten, current page or all, preview with before/after lines) | Language rules change the double quotes only | M | P2 |
| CE-133 | Transform styles: rules such as "if property `color` is `red` change it to `green`", "multiply `font-size` by 2", make family X italic | DONE | Tools → Styles → Transform Styles… (rule builder, saved rule sets, preview); new engine rules ChangeValue, SetProperty, AddProperty (make family italic) | none | L | P2 |
| CE-134 | Remove unused CSS rules | DONE | Tools → Styles → Remove Unused CSS Rules… (preview first) | Only removes rules; sheets are never deleted | M | P2 |
| CE-135 | Remove unused images / media ("Delete unused media") | DONE | Tools → Media and Fonts → Delete Unused Media… (list with sizes and ticks) | none | S | P2 |
| CE-136 | Fix HTML (per file or all files): HTML5-parse then serialise as XML | DONE | Tools → Text → Fix HTML… (open page or all pages; HTML5 parser rebuild when the quick repair fails, or always) | The HTML5 route writes attributes in sorted order | M | P1 |
| CE-137 | Beautify current file / all files (HTML, CSS, XML; only block tags containing block tags to keep whitespace safe) | DONE | Tools → Text → Beautify… (pages, stylesheets, package files; only white space between block tags changes) | No “beautify on open” preference | M | P2 |
| CE-138 | Set semantics (guide / landmarks: cover, foreword, dedication, TOC ... pointing at a location) | DONE | Tools → Structure and Accessibility → Set Semantics… (a file per kind, shows the current values) | Targets are files, not anchors inside them | M | P3 |
| CE-139 | Accessibility tool with three tabs: Language, Image descriptions, Accessibility metadata; "revert with checkpoint" | DONE | Tools → Structure and Accessibility → Accessibility… with tabs Language, Image descriptions (decorative flag, same text for every use) and Accessibility metadata (EPUB 3) | No image preview in the list; SVG images are not listed | L | P2 |
| CE-140 | Filter style information (remove listed properties from stylesheets, `<style>` and inline styles, show a summary) | DONE | Tools → Styles → Filter Style Information… (properties with counts) | none | M | P2 |
| CE-141 | Upgrade book internals (EPUB 2 to EPUB 3) | DONE | `Tool::UpgradeToEpub3` -> `upgrade::upgrade_to_epub3` | options `remove_ncx` etc. not exposed; no downgrade exists in Calibre either | - | P1 |
| CE-142 | Arrange into folders by type (Tools, Arrange into folders) | DONE | `Tool::ArrangeFiles` -> `links::arrange_files` | none | - | P2 |
| CE-143 | Check external links (visit every external URL, report broken) | DONE | Tools → Links → Check External Links… (background thread, one request every 0.4 s, progress, cancel by closing; broken links go to the results list; refuses when the network is off) | Real network not exercised in the tests: they use `BOOKX_EDITOR_NET_MOCK` | M | P3 |
| CE-144 | Download external resources (images/CSS referenced by http URL are fetched and re-linked) | DONE | Tools → Media and Fonts → Download External Resources… (lists the URLs, downloads in the background, then one undoable step) | Licence reminder in the dialog text only | M | P3 |
| CE-145 | Repair broken internal links | DONE | Tools → Links → Repair Broken Links… | Not offered from the Check list | S | P2 |
| CE-146 | Move file to another folder with link update | DONE | Tools → Links → Move Files to a Folder… (selected files, folder inside the content folder, links updated) | Not in the rename dialog | S | P3 |

### 2.6 Check book (`Tools, Check book`) and the error classes

UI behaviour first, then one row per Calibre error class. "Bookx rule" names the validator id (`validate::RULES`) or the `core::check` message that detects the same problem.

| ID | Calibre feature / error class | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-160 | Problem list; click shows help text; double-click opens the error location | DONE | results panel, `ResultActivated` jumps to file and line | Problems panel (`src/ui/editor/check_panel.rs`): grouped by file, severity icons, count badges, help pane per id (`bookx_editing::problems::help`), double-click/Enter opens file and line:column | M | P1 |
| CE-161 | Per-error "auto-fix" button where possible (checkpoint first) | DONE | one global action `Tool::FixProblems` -> `fixes::fix_book` (`FixKind`: Mimetype, Container, ManifestAdded/Removed, DuplicateManifestId, DuplicateElementId, MissingAlt, WellFormedness, Metadata, UniqueIdentifier, Version) | `problems::fix` (`fix_label`, `apply_fix`, scope file or kind); always a checkpoint first, list re-checked | L | P1 |
| CE-162 | Skip problems of this type; list of skipped types at the bottom; parse errors cannot be skipped | DONE | none | ignored kinds per book in `editor.json` (`ignored_checks`), restore list in the header; RSC-016 and package errors cannot be ignored | M | P2 |
| CE-163 | Next / previous error (Ctrl+F7, Ctrl+Shift+F7) | DONE | none | F8 / Shift+F8 (capture phase so they work in the code view) and Tools menu; Calibre's Ctrl+F7 not used | S | P2 |
| CE-164 | Option "Check for accessibility problems" | DONE | `validate` returns Usage-level ACC items (dropped by `check_all`) | checkbox in the panel header, persisted (`check_accessibility`); ACC-901..905 only then | S | P2 |
| CE-165 | Check re-run keeps list position after fixes / tracks stale entries | DONE | list is rebuilt wholesale | selection kept by (id, file, text) across re-runs; entries of edited files are dimmed until the next check | S | P3 |
| CE-166 | Malformed HTML markup (`HTMLParseError`, `XMLParseError`) | DONE | validator `RSC-016`, `core::check_book` "XML error" with line | no "HTML5 would recover to X" diff preview | - | P1 |
| CE-167 | `EmptyFile` | DONE | none | `BKX-001`, fix deletes the file | S | P3 |
| CE-168 | `DecodeError` (file not decodable) | DONE | none (`archive.get_string` is lossy) | `BKX-002`, fix converts from Windows-1252 | S | P2 |
| CE-169 | `NonUTF8` (declared encoding other than UTF-8) | DONE | `CSS-003` for CSS only | `BKX-003` (XML/HTML declaration, UTF-16), fix transcodes (`encoding_rs`); an ERROR like epubveri's RSC-028 | S | P2 |
| CE-170 | `PrivateEntities`, `NamedEntities` (named entities like `&nbsp;` in XHTML), `BadEntity` | DONE | undefined entities surface as `RSC-016` (parser error) | `BKX-004` named, `BKX-005` bad, `BKX-006` private entity; fixes; the validator's entity table is now the full HTML 4 set (`validate/entities.rs`) | S | P2 |
| CE-171 | `EscapedName` (unsafe characters in a file name) | DONE | `PKG-009`, `PKG-010`, `PKG-012` | covered by `PKG-009`/`PKG-010` (+`PKG-012` usage); new Fix renames with link repair. No new id | S | P2 |
| CE-172 | `TooLarge` (HTML over the size limit) | DONE | none | `BKX-007` above 260 KB; no auto-fix (help points to Split at Headings) | S | P3 |
| CE-173 | `BadNamespace` | DONE | `HTM-901`; fix `fixes::fix_missing_namespace` | none | - | P2 |
| CE-174 | `DuplicateId` | DONE | `HTM-905`; fix `fix_duplicate_element_ids` | none | - | P1 |
| CE-175 | `InvalidId` (id not a valid NCName-like) | DONE | none | `BKX-008`, fix renames the id and links to it | S | P3 |
| CE-176 | `DanglingIdReference` (aria-labelledby/describedby to missing id) | DONE | `HTM-909` | none | - | P3 |
| CE-177 | `BareTextInBody` | DONE | none | `BKX-009`, fix wraps loose text and inline neighbours in `<p>` | S | P3 |
| CE-178 | `CSSParseError`, `CSSError`, `CSSWarning` (stylesheets, `<style>`, inline style attributes; CSS 2.1 + some CSS 3) | DONE | `CSS-008`, `CSS-001`, `CSS-902` (`validate/css.rs`, scans stylesheets and `<style>` blocks) | `BKX-010` malformed declaration (ERROR, as epubveri CSS-008), `BKX-011` unknown property; stylesheets, `<style>`, inline `style` | M | P2 |
| CE-179 | `InvalidFont` (font cannot be processed) | DONE | `fonts::font_info` returns `None` for unparsable fonts | `BKX-012` (WOFF/WOFF2 skipped: not parsed) | S | P3 |
| CE-180 | `NotEmbeddable` (OS/2 fsType flags forbid embedding) | DONE | none | `BKX-013` (OS/2 fsType restricted) | S | P3 |
| CE-181 | `FontAliasing` (CSS `@font-face` family differs from the font's internal name; preview cannot show it) | DONE | `fonts::font_face_declarations` + `font_info` have both names | `BKX-014` usage hint; no fix (rewriting every use of the family is risky) | M | P3 |
| CE-182 | `InvalidImage` (cannot decode) | DONE | `images::decode` | `BKX-015`: header parse plus PNG/JPEG end markers (no full decode, for speed) | S | P2 |
| CE-183 | `CMYKImage` (breaks Adobe Digital Editions) | DONE | `images::decode` knows the colour type | `BKX-016`, fix converts to RGB (image crate decodes CMYK) | M | P3 |
| CE-184 | `BadLink` (link target missing) | DONE | `RSC-007`, `core::check_book` "Broken link" | fix: `links::repair_broken_links` (CE-145) | - | P1 |
| CE-185 | `FolderLink`, `MalformedURL`, `InvalidCharInLink` (":" in names), `FileLink` (`file://`), `EmptyLink` | DONE | `links::classify_link`, `validate_url` exist | `BKX-017` folder, `BKX-018` malformed (ERROR), `BKX-019` invalid char (fix encodes), `BKX-020` file: (ERROR), `BKX-021` empty | S | P3 |
| CE-186 | `CaseMismatch` (link case differs from the real file; fixable) | DONE | none | `BKX-022`, fix = `links::repair_broken_links` | S | P2 |
| CE-187 | `BadDestinationType` (link to a non-text file) | DONE | none | `BKX-023` (ERROR, as epubveri RSC-010) | S | P3 |
| CE-188 | `BadDestinationFragment` | DONE | `RSC-012`; `core::check_book` "Link target #x not found" | none | - | P1 |
| CE-189 | `LocalLink` (link outside the book) | DONE | `RSC-026` | none | - | P3 |
| CE-190 | `UnreferencedResource` (file not used by any document) / `UnreferencedDoc` (HTML not in spine) | DONE | `reports::images_report.references`, `links_report.missing_resources` | `BKX-024` unreferenced resource (fix deletes), `BKX-025` document not in the spine (fix adds it) | M | P2 |
| CE-191 | `Unmanifested` (file not in manifest; fixable) | DONE | `OPF-003`, core "File is not listed in the manifest"; fix `fixes::add_missing_manifest_entries` | none | - | P1 |
| CE-192 | `DanglingLink` | DONE | `RSC-007`/`RSC-001` | none | - | P2 |
| CE-193 | `Bookmarks` (calibre_bookmarks.txt left in the book) | DONE | n/a | `BKX-026`, fix deletes | S | P3 |
| CE-194 | `MimetypeMismatch` (declared vs actual type; fixable) | DONE | `OPF-029`, `OPF-035` | no fix wired | S | P2 |
| CE-195 | `BadOPFRoot`, `MissingOPFVersion` | DONE | `OPF-001` | none | - | P2 |
| CE-196 | `MissingSection` (metadata / manifest / spine absent) | DONE | `OPF-901..903`, `OPF-033` | `BKX-027` (ERROR; no fix) | S | P3 |
| CE-197 | `EmptyID`, `NoHref` (manifest entry without id/href) | DONE | `OPF-907`; fix `fixes::remove_missing_manifest_items` (partially) | none | - | P2 |
| CE-198 | `IncorrectIdref` (spine idref unknown) | DONE | `OPF-049`; core "Spine references unknown manifest id" | no fix | S | P2 |
| CE-199 | `IncorrectCover` / `MultipleCovers` / `NookCover` (cover meta problems) | DONE | `OPF-915` | `BKX-028` multiple covers, fix keeps the first; Nook order quirk not checked | S | P3 |
| CE-200 | `IncorrectToc` (spine toc points nowhere), `MissingNCXRef` | DONE | `OPF-050`, `OPF-910` | no fix wired in the UI | S | P2 |
| CE-201 | `MissingNav`, `EmptyNav` (EPUB 3 nav) | DONE | `OPF-912`, `OPF-913`, `NAV-901`, `NAV-001` | none | - | P2 |
| CE-202 | `MissingHref` (manifest file missing; fixable) | DONE | `RSC-001`; fix `remove_missing_manifest_items` | none | - | P1 |
| CE-203 | `NonLinearItems` | DONE | `OPF-918` (info) | none | - | P3 |
| CE-204 | `DuplicateHref` (item twice in manifest/spine; fixable) | DONE | `OPF-074`, `OPF-034`, `OPF-906`; fix `fix_duplicate_manifest_ids` | none | - | P2 |
| CE-205 | `NoUID`, `EmptyIdentifier` (fixable) | DONE | `OPF-048`, `OPF-030`, `OPF-901`; fix `fix_unique_identifier` | empty-but-present identifier not flagged separately | S | P3 |
| CE-206 | `BadSpineMime` | DONE | `OPF-042` | none | - | P3 |
| CE-207 | Accessibility `MissingAltText` (img, SVG) | DONE | `HTM-907`; fix `fixes::fix_missing_alt`; `accessibility::alt_text_report` | `ACC-905` inline SVG without title/label (with accessibility option); img alt = `HTM-907` | M | P2 |
| CE-208 | Accessibility `MissingLanguage` (file lang), `MissingPackageLanguage` | DONE | `ACC-007` (usage), `OPF-903`; fix in `accessibility::make_accessible` | shown only with CE-164 | - | P2 |
| CE-209 | Accessibility `SkippedHeadingLevel` | DONE | none | `ACC-901`, no fix | S | P3 |
| CE-210 | Accessibility `MissingRole` (epub:type with a DPUB role but no `role`; fixable) | DONE | `accessibility::epub_type_to_role` + `make_accessible` adds roles | `ACC-902`, fix adds roles | S | P3 |
| CE-211 | Accessibility `MissingAccessibilityMetadata`, `MissingAccessibilitySummary` | DONE | `make_accessible` adds metadata | `ACC-903`, `ACC-904`, fix = `make_accessible` | S | P3 |
| CE-212 | Check that the book still has the old/new validity (epubveri-grade validation beyond Calibre) | DONE | `validate` (87 rules) merged in `check_all` | Bookx exceeds Calibre here | - | P1 |

### 2.7 Checkpoints and Compare (`diff.html`)

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-220 | Create checkpoint (Edit, Create checkpoint) | DONE | Tools, Checkpoints… dialog: named checkpoint (`PvMsg::CheckpointNew`); the old menu entry keeps auto names | none | S | P2 |
| CE-221 | Automatic checkpoint before every tool | DONE | `run_op` -> `create_checkpoint("Before: …")`, rolled back if the tool fails | none | - | P1 |
| CE-222 | Revert to a checkpoint (View, Checkpoints, Revert to) | DONE | `RestoreCheckpoint` (keeps a "Before: Restore" snapshot) | none | - | P1 |
| CE-223 | Checkpoints list dialog (View, Checkpoints) with Compare and Revert buttons | DONE | Checkpoints dialog (`compare.rs`): time, file count, size, keep (pin), rename, Compare, Revert, delete; refreshes while open | cap is still 25 | M | P2 |
| CE-224 | Compare view: side-by-side diff, only differences with context, green added / red removed / blue changed, line numbers | DONE | Compare page in the editor centre: two synchronized views, line numbers, folded context, added/removed/changed tints, word marks, dark and e-ink tags (`compare::side_by_side`) | not a dialog but a page of the editor; binary files are not shown | L | P1 |
| CE-225 | Compare: double-click a line to go to it in the editor | DONE | double click a row goes to the line in the open book (`CmpMsg::Goto`) | with Beautify on the line numbers are of the pretty-printed text | S | P2 |
| CE-226 | Compare options: beautify before comparing, context lines; search bar (left/right) | DONE | Beautify toggle, lines of context, search box with next, next/previous change | no left/right-only search | S | P3 |
| CE-227 | Compare to another book file (File, Compare to other book) | DONE | Tools, Compare with Another Book… (EPUB file chooser) opens the same view | EPUB only | M | P3 |
| CE-228 | Compare ORIGINAL_FMT to FMT after a conversion | NOT PLANNED | Bookx keeps no ORIGINAL_ format; Compare with Another Book… against the original file covers it (documented in the wiki) | needs the catalogue to store originals | M | P3 |
| CE-229 | Compare via the Book details panel (library window) | NOT PLANNED | the catalogue does not store originals, so there is nothing to compare with in the library | see CE-228 | M | P3 |

### 2.8 Live preview and splitting

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-240 | Live preview updates after edits when idle | DONE | `schedule_preview`, `refresh_preview_now` (uses Bookx's own `BookCanvas`, theme/original/custom colours popover) | none | - | P1 |
| CE-241 | Auto reload preview toggle | DONE | preview header toggle “Reload automatically”; when off the refresh button turns amber (stale) | none | S | P2 |
| CE-242 | Refresh preview (F5) | DONE | F5 and the refresh button (`PvMsg::Refresh`) | none | S | P2 |
| CE-243 | Sync preview position to editor cursor ("auto sync" toggle) | DONE | `bookx_editing::preview_map` pairs source segments with the preview blocks (text based, no parser change); blocks get anchors `bxsN`; Follow cursor toggle (on by default); 1.5 MB chapter maps in a perf budget | vertical text: scrolls but no marker; blocks the preview generates have no place | L | P1 |
| CE-244 | Clicking in the preview moves the editor cursor to the element | DONE | click in the preview: `BookCanvas::block_at_point` → caret at the element (`canvas_src.rs`); real-input session scenario | places the caret, does not select the element | L | P1 |
| CE-245 | Clicking an in-book link in the preview opens that file in editor and preview | DONE | `on_link` → `open_location`; external links are only named | none | S | P2 |
| CE-246 | "Pick a location" mode of the preview (used by ToC editing and splitting) | DONE | `Pick::{Toc,Split}` mode: banner, crosshair, green marker line following the pointer, Cancel | horizontal text only for the marker | L | P2 |
| CE-247 | Copy selection from preview (Ctrl+Alt+C), find in preview | DONE | copy button / Ctrl+Alt+C, find in the preview with next/previous | none | S | P3 |
| CE-248 | Split mode in the preview: click a position, the file is split there and links are migrated | DONE | Markup menu: Split at Cursor (`SplitSpec::AtOffsets`, `snap_offset`) and Split by Picking in the Preview; links, spine, ToC migrated by `split_file`; real-session scenario | none | M | P1 |
| CE-249 | Split at multiple locations (all headings, all tags with a class, XPath) | DONE | Split at Several Places… dialog: headings by level, class (+tag), page breaks, XPath-like selector (`split_merge::Selector`), live list of resulting files | selector subset: steps, `[@a]`, `[@a='v']`, `[contains(@a,'v')]`, `tag.class`, `#id` | M | P1 |

### 2.9 Live CSS panel and code inspector

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-260 | Live CSS panel: tag name + line, all matching rules, inherited rules, clickable origins that jump to the rule, computed values, overridden properties struck through (View, Live CSS) | DONE | sidebar tab Live CSS: matched rules in cascade order with file:line (click jumps), inherited rules by ancestor, resulting values, overridden declarations struck through (`bookx_editing::live_css`) | declared values after the cascade, no user-agent/reader defaults, em/% not resolved; read only | XL | P2 |
| CE-261 | Code inspector (Inspect in the preview: element HTML and styles, experimental style editing) | NOT PLANNED | Live CSS covers the need; no browser inspector | none | XL | P3 |

### 2.10 Miscellaneous tools

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-270 | Check spelling (Tools, Check spelling): list of misspelled words with count and language | DONE | `ui/editor/spelling.rs` side panel (Tools → Check Spelling): list with count and language, scan on a worker thread (`spell_session::scan_sources`) | none | L | P1 |
| CE-271 | Language aware per element (`lang`/`xml:lang`) with multiple dictionaries (en_US vs en_GB) | DONE | `SpellChecker` keyed by full tag (en-GB / en-US side by side, fallback to bare language then main variant); `load_languages` loads a dictionary per language found; `canonical_tag` | none | M | P1 |
| CE-272 | Double-click a word: highlight next occurrence in the editor | DONE | selecting a row selects the next occurrence after the caret (focus stays in the list; Enter/double-click focuses the editor) | none | S | P2 |
| CE-273 | Change selected word to a chosen suggestion or typed word everywhere ("Change all") | DONE | Change All: suggestion list or typed word, `spell_session::change_everywhere` through `run_op` (checkpoint) | none | M | P1 |
| CE-274 | Ignore (session) | DONE | Ignore (session) and Ignore in Book (`spelling-books/<id>.json`) | none | S | P1 |
| CE-275 | Add to dictionary; multiple user dictionaries | DONE | named user dictionaries (`spell_user`: `spelling-words.txt` = Personal, `spelling-dictionaries/*.txt`, enable/disable, add/remove words) chosen in the panel; managed in *Spelling Dictionaries…* | none | M | P1 |
| CE-276 | Show all words (not only misspelled) with counts to find/replace single words | DONE | "all words" toggle with counts, filter field | none | S | P3 |
| CE-277 | Refresh button when the book was edited while the tool is open | DONE | Refresh button; scans read the open buffers, so unsaved edits count | none | S | P3 |
| CE-278 | Exclude files (button or `<!-- calibre-no-spell-check -->` comment) | DONE | panel menu *Exclude This File* / *Excluded Files…* (per book) and the comments `bookx-no-spell-check` and `calibre-no-spell-check` | none | S | P3 |
| CE-279 | Add dictionaries (LibreOffice `.oxt`) via Preferences, Manage spelling dictionaries | DONE | *Spelling Dictionaries…*: lists system + user dictionaries, imports a `.dic`/`.aff` pair or `.oxt` into `~/.local/share/Bookx/hunspell` (`BOOKX_HUNSPELL_USER`), removes imported ones; UTF-8 dictionaries only | only UTF-8 dictionaries | M | P2 |
| CE-280 | Next spelling mistake in the editor (F8); misspelled words underlined in the code view (setting) | DONE | underlines + `pick_spot`; Ctrl+F8 / Ctrl+Shift+F8 (plain F8 is the Problems panel) | F8 itself taken by CE-163 | M | P2 |
| CE-281 | Check grammar (Shift+F7, neural nets, rewrite suggestions) | NOT PLANNED | none | **do not copy** (section 5); stated on the wiki | XL | P3 |
| CE-300 | Insert special character: dialog of all Unicode characters, Ctrl-click closes after insert, favourites (right-click to add/remove, re-arrange), works in any text field | DONE | `ui/editor/special_chars.rs`: non-modal window, all blocks of `charinfo::BLOCKS` (named, assigned characters only), search by name/code, favourites (add/remove/move, `special-chars.json`), recent; Ctrl-click keeps it open. Uses `unicode_names2` | none | M | P2 |
| CE-301 | Type a hex code then Alt+X to convert to the character | DONE | Alt+X key handler on the code view (`charinfo::alt_x`, `U+` prefix too); real-session scenario `editor_alt_x` | none | S | P3 |
| CE-302 | Typing `&nbsp;` with `;` is replaced by the entity's character | DONE | setting *Convert Typed Entities* (off by default, `editor.json`); only the ~30 entities of `markup::named_entity` plus numeric ones; `&amp; &lt; &gt; &quot; &apos;` never converted | none | S | P3 |
| CE-303 | Reports tool (Tools, Reports): Files, Words, Images, Style rules, Style classes, Characters, Links; every row hot-linked; sortable; CSV export | DONE | `ui/editor/reports.rs`: AdwViewSwitcher window with 7 tabs, sortable/filterable ColumnViews, jump on activate; `reports_tables` | none | L | P2 |
| CE-304 | Report "Files": folder, name, size (KB), type, word count; mark/delete selected files | DONE | Files tab with folder, name, size, type, words (`FileEntry.words`); Delete Selected | none | M | P2 |
| CE-305 | Report "Words": word, language, times used; filter, jump to occurrences | DONE | Words tab: full list (not top N), language, count, first 200 places, Next Place | none | M | P2 |
| CE-306 | Report "Images": image, size KB, times used, resolution; delete unused | DONE | Images tab with times used and resolution; Delete Unused | none | M | P2 |
| CE-307 | Report "Links": source, source text, target, anchor, target text, ok flag; double-click source / target | DONE | Links tab: source, text, target, anchor, kind, status; activate = source, Go to Target = target | none | M | P2 |
| CE-308 | Report "Characters": character, name, code point, times used | DONE | Characters tab with Unicode name and places | none | M | P3 |
| CE-309 | Report "Style rules": rule, number of matches, jump to rule; sort | DONE | Style rules tab with matches and line in the stylesheet | none | M | P2 |
| CE-310 | Report "Style classes": class, number of matches | DONE | `reports_tables::class_usage`: use counts, used / unused / not defined | none | M | P3 |
| CE-311 | Reports: export (CSV) of a view | DONE | CSV export of the rows shown (`Table::to_csv`, formula-safe) | none | S | P3 |

### 2.11 Special features in the code editor

*Updated 2026-11-12 for the switch to GtkSourceView (`docs/enhancements.md` item 5): the code view is a `sourceview5::View`; rows below say what each feature uses.*

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-330 | Syntax highlighting for HTML/XML (tags, attributes, strings, comments, entities) | DONE | GtkSourceView `html`/`xml` language (`codeview::apply_language`); `syntax::scan_xml` stays as the fallback when no language is found | none | - | P1 |
| CE-331 | Syntax highlighting for CSS (selectors, properties, strings, comments) | DONE | GtkSourceView `css` language (values, at-rules, `!important` included); `syntax::scan_css` is the fallback | none | S | P2 |
| CE-332 | CSS inside `<style>` tags highlighted | DONE | GtkSourceView's `html` language highlights `<style>` and `<script>` blocks as CSS/JS (not covered by a UI scenario) | none | S | P2 |
| CE-333 | Bold/italic text and heading text rendered bold/italic in the code view | DONE | View menu, *Draw Bold and Italic Text* (off by default, `editor-markup.json`): text tags `code-bold` / `code-italic` over the content of `b`, `strong`, `th`, `h1`-`h6` and `i`, `em`, `cite`, `dfn` (`code_marks::emphasis_spans`, `ui/editor/markup_hints.rs`); counts checked by scenario `editor_code_hints` | proportional weight in a monospace font changes the column width a little; off by default for that reason | M | P3 |
| CE-334 | Matching tag highlighting when the cursor moves | DONE | live highlight of the open/close tag at the caret (`tags::highlight_pair`, `live::update_pair_highlight`, tag `tag-pair`); scenario `editor_live_pairs`; selecting with Ctrl+Alt+K as before | none | M | P1 |
| CE-335 | Jump to opening/closing tag (Ctrl+{ / Ctrl+}); select tag contents (Ctrl+Alt+T / Ctrl+Shift+T) | DONE | Edit menu, Jump to Matching Tag (Ctrl+Alt+J); select to matching (Ctrl+Alt+K) | shortcuts differ from Calibre’s | M | P1 |
| CE-336 | Invalid HTML underlined in red | DONE | red squiggle (tag `markup-error`) from a debounced pass (`code_marks::markup_errors`, `css_errors`): unclosed or stray tags, unfinished tags, duplicate and unquoted attributes, a bare `&`, unclosed comments (XHTML strict; `.html` only the unfinished-tag and duplicate-attribute checks), unbalanced braces in CSS; tooltip with the reason; identical results are not re-applied (no flicker); files over 1.5 MB are skipped; View menu *Underline Invalid Markup*; scenarios `editor_code_hints`, `editor_code_marks_visual` (real session, pixel check) | not a full XML validation (Tools → Check Book does that); entity names are not looked up | M | P2 |
| CE-337 | Spelling errors underlined (language aware) | DONE | squiggles from the Hunspell engine (`editor_spell`, debounced pass, window ± 40 lines, tag `spell-error`); right-click: Replace with…, Ignore, Add to Dictionary; gutter count; scenario `editor_live_spell` | only the book language; files over 700 kB are not checked live | M | P2 |
| CE-338 | Highlight hard-to-see characters (nbsp, different hyphens, zero-width) | DONE | Edit menu, *Show Hidden Characters*: GtkSourceView space drawer (spaces, tabs, line breaks) plus a yellow highlight for no-break/zero-width/soft-hyphen characters (`codeview::hidden_chars`); the status line names the one under the caret; scenarios `editor_view`, `editor_live_hidden` | none | S | P2 |
| CE-339 | Link file names highlighted; missing target underlined red | DONE | every internal `href`/`src`/`xlink:href`/`poster`/`data` and CSS `url()`/`@import` value is underlined (`link-target`); a missing file or `#anchor` gets a red underline (`link-missing`) and a tooltip; anchors come from `Data::ids_of` (unsaved ids of other tabs included); recomputed with the live pass; View menu *Mark Link Targets*; scenario `editor_code_hints` | links inside `srcset` and `<a name>` collisions are not checked | M | P2 |
| CE-340 | Line numbers gutter | DONE | GtkSourceView line numbers (`View::set_show_line_numbers`) | none | - | P1 |
| CE-341 | Context sensitive help: right-click a tag/CSS property for docs | DONE | right-click a tag, attribute or CSS property: *Help for <x>* opens the MDN page in the browser (`code_nav::help_topic_at`, `mdn_url`; the name is restricted to `[a-z0-9-]`; only the system browser is started, the app itself makes no request); scenario `editor_code_nav` | MDN only; no offline help | S | P3 |
| CE-342 | Ctrl+click a file name in a link opens that file; Ctrl+click a class jumps to the matching style rule | DONE | Ctrl+click (capture-phase `GestureClick`) or F12 (*Go to Definition*): a link value opens its file (and the anchor), a class name goes to its first rule (style sheets the page links first) and selects the name, a selector in a style sheet goes to the first page that uses the class (`code_nav::hit_at`, `plan_jump`, `ui/editor/navigate.rs`); also inside `<style>` blocks, `url()` and `@import`; scenarios `editor_code_nav` (Broadway), `editor_code_ctrl_click` (real session: real Ctrl+click on a link, a class and a selector, F12) | no pointer cursor / underline while Ctrl is held; external links only toast; the first use is chosen (no cycling) | M | P1 |
| CE-343 | Right-click a class name: rename the class across HTML and CSS | DONE | context menu *Rename Class “x”*: dialog with the new name, the number of changes and the files with a before → after line; *Rename* takes a checkpoint first and applies the new text to every buffer (one undo step per file, saved with the book); pure `code_nav::rename_class` over every page, style sheet and `<style>` block (whole class names only, not in comments, strings or `[attr]` selectors; refuses invalid names, warns when the name exists); scenarios `editor_code_rename`, `editor_code_rename_class_dialog` (real session) | class names written in scripts, the OPF or escaped (`\.`) are not renamed | M | P2 |
| CE-344 | Jump to definitions: anchors/ids (link `#id` in file), CSS rule for a class | DONE | the same hit test: `file#id` and `#id` go to the element with that `id` (or `<a name>`), an `id=` goes to its `#id` rule and a `#id` selector to the element (`find_id_def`, `find_id_rule`, `find_id_use`) | none | M | P2 |
| CE-345 | Auto-complete for file names (subsequence matching, prioritising word starts and after `/`; Tab to accept), relative path computed | DONE | `bookx_editing::fuzzy` ranks by subsequence with bonuses for word starts, the start of the base name and consecutive letters (`c2` finds `chapter-2.xhtml`, `s.css` finds `style.css`); a plain prefix still ranks first; Tab accepts the selected proposal (GtkSourceView's own key, verified with real keys in `editor_code_complete_keys`); scenario `editor_code_complete` | none | L | P1 |
| CE-346 | Auto-complete for anchors after `#` with a text snippet of each anchor | DONE | after `file#` or `#` the ids of that file, each shown as `id  —  first words of its text` (`code_marks::anchor_snippets`; unsaved text of the open tabs is used); real popup screenshot in `editor_code_complete_keys` | the snippet is cut at 48 characters | M | P1 |
| CE-347 | Auto-complete tag names / CSS property names | DONE | tag names after `<`, attribute names per tag (inserts `name=""` with the caret inside), CSS property names in rules (`editor_complete::TAGS`, `CSS_PROPERTIES`), class names in `class="…"` from the stylesheets, words of the file as fallback | none | M | P2 |
| CE-348 | Auto-close tags when typing `</` (preference) | DONE | typing `</` inserts the name of the innermost open element and `>` (`code_format::auto_close`; not in comments, CDATA or `<script>`/`<style>`; not when that closing tag follows already); typing the rest by hand is swallowed (`type_over`), so `</p>` typed in full is not doubled; View menu *Close Tags When Typing </* (default on, `editor-markup.json`); coexists with the Preferences option that adds the closing tag when `>` is typed; scenarios `editor_code_insert`, `editor_code_format_keys` (real keys) | not wired to input-method commits | S | P2 |
| CE-349 | Smart comment toggle (Ctrl+`) | DONE | Edit menu, Toggle Comment (Ctrl+/), HTML `<!-- -->` and CSS `/* */` (`tags::toggle_comment`) | none | S | P2 |
| CE-350 | Format actions: bold (Ctrl+B), italic (Ctrl+I), underline, strikethrough, superscript, subscript, text colour, background colour, align left/centre/right/justify | PARTIAL | Markup menu → *Format*: Bold (Ctrl+B), Italic (Ctrl+I), Underline (Ctrl+U), Strikethrough, Superscript, Subscript, Code (bind keys in Preferences → Keyboard Shortcuts); the selection is wrapped in `b`/`i`/`u`/`s`/`sup`/`sub`/`code`, a selection that is exactly such an element (or inside one) toggles it off (`<b>` and `<strong>` count alike; the surrounding element is closed and reopened around a partial selection); a selection that cuts through tags is refused with a hint (`code_format::toggle_format`); scenarios `editor_code_format`, `editor_code_format_keys` (real Ctrl+B / Ctrl+I) | text colour, background colour and alignment (they need rules added to a style sheet) are not there | M | P2 |
| CE-351 | Rename block tag to h1..h6 / p (Ctrl+1..Ctrl+7) and "Change paragraph to heading" | DONE | Edit menu, Change Block to Paragraph/Heading 1-6 (Ctrl+0…6) and Rename Tag… (Ctrl+Alt+R); `tags::change_block_to_heading`, `rename_tag` | no Ctrl+7 | S | P1 |
| CE-352 | Insert tag (Ctrl+<) around selection; Remove tag (Ctrl+>); Split tag at cursor (Ctrl+Alt+>) | DONE | Wrap Selection in Tag (Ctrl+Alt+W or Ctrl+<; the name prompt now has the focus), Remove Surrounding Tag (Ctrl+Alt+U or Ctrl+>), Split Tag at Caret (Ctrl+Alt+>, the copy of the start tag loses its `id`; `code_format::split_tag`), Close the Open Tag (Ctrl+Alt+.); scenario `editor_code_tag_keys` (real keys) | none | M | P1 |
| CE-353 | Change case of selection: upper, lower, swap, title, capitalize | DONE | Edit menu, Change Case: upper, lower, title, swap (`tags::change_case`) | no “capitalize first letter” variant | S | P2 |
| CE-354 | Insert image dialog, Insert hyperlink dialog (picks a file/anchor from the book) | DONE | Markup menu / Ctrl+Alt+M and Ctrl+K: `adw::Dialog` with a filter that ranks by subsequence (`fuzzy`), images with thumbnails (decoded one per idle tick), pages with an anchor chooser (ids with text snippets), address, link text (a selection stays the link content, markup included), title, alt text and a live preview of the markup; Up/Down move in the list, Enter inserts; inside a link the address of that link changes instead of nesting; scenarios `editor_code_insert` (the markup), `editor_code_insert_dialogs`, `editor_code_dialogs_small` (900x600, real keys) | no preview of the image at full size; web addresses are typed, not validated | M | P1 |
| CE-355 | Sort style rules (CSS) | DONE | Markup menu → *Sort CSS Rules* (also in the command palette, no default key): style rules between at-rules are sorted by selector, a comment right above a rule moves with it, `@import`/`@media`/`@font-face` keep their place (`code_nav::sort_css_rules`, one undo step); scenario `editor_code_insert` | rules inside `@media` are not sorted | S | P3 |
| CE-356 | CSS colour preview in the code (underline or background in the colour) | DONE | a background swatch in the colour behind `#rgb`, `#rrggbb[aa]`, `rgb()`, `rgba()`, `hsl()`, `hsla()` and about 60 colour names (names only in colour properties, never in `font-family`, selectors, strings, comments or `url()`), in style sheets, `<style>` blocks and `style` attributes; the text on the swatch is black or white, whichever has the higher contrast (`code_marks::css_colors`, `readable_text`); View menu *Show Colour Swatches*; scenarios `editor_code_hints`, `editor_code_marks_visual` (pixel check) | a white swatch on the light scheme is invisible; the tag colours are not re-tinted for e-ink | S | P3 |
| CE-357 | Tab width, editor font family/size, cursor width, custom colour schemes | PARTIAL | PARTIAL: Preferences → Editor (`prefs_page.rs`, `editor-look.json`): font family and size (`GtkFontDialogButton`), tab width, spaces or tabs, line numbers, current-line highlight, right margin and column, colour scheme chooser (`GtkSourceStyleSchemeChooserWidget`, applied live, including schemes dropped into `<config>/styles/*.xml`), "follow light/dark" switch | cursor width and blink: GTK 4 has them only as display-wide desktop settings (`gtk-cursor-blink`, `gtk-cursor-aspect-ratio`), no per-view API, so they follow the desktop; not planned | M | P2 |
| CE-358 | Allow drag-and-drop editing of text | DONE | GTK default behaviour of `GtkTextView` | none | - | P3 |
| CE-359 | Per-file undo/redo (Ctrl+Z, Ctrl+Y) and the usual clipboard keys | DONE | per-file `GtkSourceBuffer` undo/redo (`Input::Undo/Redo`) | none | - | P3 |

### 2.12 Snippets (`snippets.html`)

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-380 | Expand a snippet: type the trigger then Ctrl+J; Ctrl+J again jumps to the next placeholder | DONE | type the trigger, press **Tab** (not Ctrl+J): `completion::expand_at_caret` pushes a GtkSourceView snippet; Tab then moves between the tab stops (a region around the expanded text tells the two cases apart); scenario `editor_live_snippets` |  | L | P2 |
| CE-381 | Placeholders `$1`, `$2` ordered; final `$N` = end position | DONE | `$1`, `$2`…, `$0` = end position (GtkSourceView snippet syntax) |  | in CE-380 | P2 |
| CE-382 | Default text `${1:filename}` selected when jumping | DONE | `${1:default}` (the default is selected while the stop is active) |  | in CE-380 | P2 |
| CE-383 | Selected-text placeholder `${2*}` / `${2*:default}`: text selected before expansion is substituted | DONE | DONE: `${2*}` / `${2*:default}`: selected text before Tab (type the trigger, select the text, Tab; or *Insert Snippet*, Ctrl+J) becomes the text of tab stop 2; the pure `with_selection` builds the GtkSourceView body (escapes `$`, `}`, `\`) | none | in CE-380 | P2 |
| CE-384 | Mirrored placeholders (same `$1` several times is filled with what you typed) | DONE | the same `$1` several times is mirrored by GtkSourceView (the built-in *Heading* uses it); not covered by a UI scenario |  | in CE-380 | P2 |
| CE-385 | Built-ins: `Lorem` (filler paragraphs), `<>` (self-closing tag), `<a` (link), `<i` (image), `<<` (any tag, wraps selection), `<c` (tag with class) | DONE | DONE: built-ins `Lorem` (three filler paragraphs), `<>` (self-closing tag), `<a` (link), `<i` (image), `<<` (any tag, wraps the selection), `<c` (tag with class); triggers may start with `<`; `p`, `h`, `link` wrap the selection too | none | S | P2 |
| CE-386 | User snippets: name, trigger, template, file types (HTML/CSS/...) and a Test box; Manage snippets dialog | DONE | DONE: *Manage Snippets*: name, trigger, applies to (all / markup / CSS), text, a **Test** box (sample selected text, expands in a scratch view with the real engine), import and export as JSON (invalid ones are skipped with the reason), templates for new files in the same dialog | finer file types than markup/CSS are not offered | M | P3 |
| CE-387 | Snippets usable in Search & replace fields (without placeholder jumping) | DONE | DONE: Tab in the Find and Replace fields expands a trigger into plain text (`flatten`: tab stops show their default text, nothing jumps; a selection fills `${N*}`) | none | S | P3 |

### 2.13 Preferences, "tweaks", integration

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-400 | Preferences window (main window, editor, preview, keyboard shortcuts, toolbars, integration with calibre) | DONE | DONE: *Editor* page in Preferences (registry) and *Editor Preferences* in the editor (palette command, bindable) opening the same groups in a dialog over the editor | main-window, toolbar and calibre-integration pages of Calibre's dialog do not apply | M | P2 |
| CE-401 | Editor preferences: font, size, tab width, cursor width, colour scheme, CSS colour preview, auto close tags, beautify on open, show misspelled words underlined, drag-and-drop editing | PARTIAL | PARTIAL: font + size, tab width, spaces, line numbers, right margin, colour scheme, CSS colour preview (CE-356), auto-close tags (typing `>`), completion while typing on/off, underline misspelled words, wrap, whitespace, minimap, entity conversion | beautify on open and drag-and-drop editing preferences are not offered | M | P2 |
| CE-402 | Preview preferences: fonts per family, minimum font size, visible lines above sync point, background/foreground/link colour overrides | PARTIAL | PARTIAL: preview font, smallest text size, follow the cursor at start, scroll both ways (plus the colours popover in the preview header) | "lines above the sync point" and per-family fonts (serif / sans / mono) are not offered: the preview uses one font | M | P3 |
| CE-403 | Customisable keyboard shortcuts (all actions) | DONE | DONE: every editor action is in the shortcut registry (group *Editor*, about 60 actions with defaults); Preferences → Keyboard Shortcuts lists them and rebinds with conflict handling (swap or take away); applied live to every editor window (`ui/editor/keymap.rs`), shown in the command palette; Editor keys never clash with the library or reader keys | menu items do not show the accelerator text (they are `GMenu` items without actions of their own); the palette and Preferences do | L | P2 |
| CE-404 | Customisable toolbars per file type | NOT PLANNED | not planned: the editor has no toolbars to customise; its commands are in the menus, the Edit/Tools/Files popovers and the command palette (Ctrl+Shift+P), which are searchable and show the (rebindable) shortcut. Per-file-type toolbars conflict with the adaptive libadwaita layout (section 5) | - | - | P3 |
| CE-406 | Templates for new files | DONE | DONE: *Manage Snippets* has a *Templates for new files* section (add, edit, delete; XHTML or CSS, `{title}`); they are the files of `editor-templates/` that *New File* offers | none | S | P3 |

### 2.14 Features in the Calibre source that are not on the manual page

| ID | Calibre feature | Bookx status | Where in Bookx | What is missing / design note for Bookx | Effort | Prio |
|---|---|---|---|---|---|---|
| CE-420 | Manage fonts (list, change family, remove) | DONE | DONE: Tools → Media and Fonts → Manage Fonts: embedded fonts with family, style, size, characters used, missing glyphs and licence flag; rename a family everywhere (`@font-face`, `font-family`, `font`, style attributes; the case-insensitive match of `transform_css` stays), remove fonts with their `@font-face` rules, a vector sample of the real glyphs (`fonts_manage::sample_svg`, sample text editable), embed from the computer opens Embed Referenced Fonts. `fonts_manage.rs` (5 unit tests), `fonts_manager.rs`; tests: `editor_struct_fonts`, `editor_struct_dialogs` | the sample is an SVG made from the outlines (no hinting, no kerning) | M | P3 |
| CE-421 | Transform HTML (rule-based element/attribute changes) | DONE | DONE: Tools → Styles → Transform HTML: rename tag, unwrap (remove tag keep content), remove element, set / remove / replace-text attribute, wrap, add / remove / change class; selectors of the split tool (`p.note`, `#id`, `//section/p`, `[@epub:type=…]`); all pages or the open one; Preview lists the changed pages; saved rule sets (`editor-html-rules/`); the engine edits the source text in place, so entities, quotes, comments and namespace prefixes stay as written; broken pages are skipped and listed. `bookx_editing::transform_html` (15 unit tests), `html_transform_ui.rs`; tests: `editor_struct_html` | no regex on text content (use Search and Replace); no "wrap range of siblings" | L | P3 |
| CE-422 | Add text-to-speech narration (media overlays) | DONE | Tools → Add Narration / Remove Narration: the local read-aloud engines (Kokoro, Piper, voice server) read the EPUB 3 chapter by chapter into WAV files, sentences get `<span id="bxsN">`, SMIL files with `clipBegin`/`clipEnd`, manifest items, `media-overlay`, `media:duration`, `media:active-class`; progress dialog + Jobs panel, cancel keeps finished chapters, rerun resumes, one checkpoint; validation rules `MED-001..012` inside `validate()`; the reader plays a book that has its own narration (WAV) with the read-aloud card and highlight (`src/overlay_play.rs`) | `bookx-editing` `media_overlay*.rs` (about 50 unit tests incl. malformed/hostile SMIL), `src/overlay_play.rs` and `src/narration.rs` unit tests, scenarios `editor_narration`, `reader_narration`, live test `narration::tests::live_a_short_chapter_with_kokoro` (Kokoro int8: 8.3 s of audio in 0.9 s, RTF 0.11, not listened to). Limits: **WAV only** (no pure-Rust Opus/MP3/AAC encoder in the workspace; WAV is not an EPUB 3 core media type, rule MED-011; about 48 KB/s), playback decodes only 16-bit PCM WAV, a sentence is split at inline tags, partial chapter is dropped on cancel | XL | P3 |
| CE-423 | Remove unused images | DONE | DONE (see CE-135): Tools → Media and Fonts → Delete Unused Media lists unused images, fonts, audio and video with sizes (the cover is never listed); `book_tools::unused_media`, scenario `editor_x_media` checks the images (`orphan.png` listed and deleted, `used.png` kept) | none | S | P2 |
| CE-424 | Fuzzy search mode | DONE | DONE: *Fuzzy* in the mode dropdown: approximate matches (edit distance grows with the length of the query: none up to 3 characters, 1 up to 6, then 1 more per 4); case, whole word, marked-text scope and Replace All work (`editor_search_fuzzy.rs`) | none | M | P3 |
| CE-425 | Mark selected text in the editor for search | DONE | DONE: Ctrl+Shift+M marks the selection (tinted in the text and in the minimap), the scope "Marked text" searches only there, *Clear Marked Text* (palette command, bindable; Ctrl+Shift+M with nothing selected too) clears all files; marks live in the buffers, so they survive tab switches | a marker in the plain scrollbar is not drawn (the minimap shows them) | M | P3 |
| CE-426 | Book jacket (add/remove metadata jacket page) | DONE | DONE: Tools → Structure and Accessibility → Book Jacket: add / update / remove a marked page (`<meta name="bookx:jacket" content="cover=1 series=1 …">`, so the options are remembered) after the cover page with title, authors, series, rating, tags, comments, optional cover and publisher; saving metadata in the editor refreshes it; removal leaves no file, manifest or spine entry behind. `bookx_editing::jacket` (7 unit tests), `jacket.rs`; tests: `editor_struct_jacket` | the jacket is plain XHTML with a small inline style, not Calibre's template | M | P3 |
| CE-427 | EPUB 3 accessibility metadata form | DONE | DONE: Tools → Structure and Accessibility → Accessibility Metadata: summary, access modes, sufficient modes (sets such as `textual,visual`), features, hazards, `conformsTo` (standard statements), certifier (`a11y:certifiedBy`, `a11y:certifierCredential`, `a11y:certifierReport`); three presets; *Suggest from the Book* (images and their alt text, audio/video, MathML, ARIA roles, page breaks, headings, contents, fixed layout); live validation (errors stop Apply) and a check of the claims against the book (`alt_text_report`). `bookx_editing::a11y_meta` (9 unit tests), `a11y_form.rs`; tests: `editor_struct_a11y`, `editor_struct_dialogs` | flashing and motion hazards cannot be detected (the form says so); no external checker such as Ace | L | P2 |
| CE-428 | Edit metadata in the editor | DONE | "Metadata" tab, `metadata::edit_dialog`, `ApplyMetadata` | OPF source stays editable via the file list | - | P1 |
| CE-429 | Edit formats other than EPUB (AZW3/KEPUB/HTMLZ) via unpack/repack | DONE | DONE: Book menu → Open a Book of Another Format to Edit: AZW3/AZW, MOBI/PRC, FB2, FBZ, DOCX, ODT, RTF, HTMLZ, TXTZ are converted to a temporary EPUB in memory (`bookx_formats`), KEPUB is edited as it is with its Kobo spans; nothing enters the library. Save asks: Save as EPUB, or Convert Back to the format (`bookx_convert` writers; the old file is kept as `name.ext.bak`, `.bak2`, …; atomic write; nothing is replaced when the conversion fails); a warning names what may be lost per format. `editor_structure.rs` (open/save-back, 8 unit tests), `unpack_repack.rs`; tests: `editor_struct_foreign` | ODT and FBZ cannot be written back (EPUB only); converting back is a conversion and can lose styling; untested with real Kindle/Kobo hardware | XL | P3 |

## 3. Recommended implementation order (waves)

Dependencies are written as "needs". "UI" = only GTK wiring on existing engines; "engine" = new work in `bookx-editing`/core first; "new subsystem" = larger.

**Wave 0 - fix and protect what exists (UI + tiny engine, 1 week)**
1. CE-023 rename through `markup::rename_paths` (+ regression test: renaming `a.png` leaves `data.png` alone), CE-034/CE-007 checkpoints for delete, move, replace all, ToC generate.
2. CE-072/073/074 scope names (text files / style files / all files), CE-080 go to line, CE-061/062 shortcuts. All UI, plus `build-aux/ui-tests.sh` scenarios.

**Wave 1 - editing core on the existing buffer (UI, small engine pieces, no new crate)**
3. Tag-pair scanner in `bookx-editing` (pure, from `markup::scan`) -> CE-334, CE-335, CE-351, CE-352, CE-349, CE-348, CE-353, CE-338, CE-332. Needs: nothing else.
4. Regex engine decision and replacement dialect (CE-066, CE-067, CE-070) - **blocks** saved searches and function mode (Wave 4).
5. Multi-select file list (CE-026) -> merge many (CE-027), delete (CE-034), bulk rename (CE-024/025), export/replace (CE-035/036), link stylesheets (CE-041), new file dialog (CE-039). Needs the Wave 0 rename.
6. ToC: navigate on select (CE-126), generate options (CE-121/122/123), drag/drop (CE-120). ToC pick-in-preview (CE-124) waits for Wave 3.

**Wave 2 - expose finished engines (UI only, one dialog each)**
7. Spelling dialog (CE-270..CE-275, CE-279, CE-280): needs user word list persistence only.
8. Reports window (CE-303..CE-311): `reports::build_report` is complete; add per-class counts (CE-310) and per-file word count (CE-304) in the engine.
9. Filter styles (CE-140), Transform styles (CE-133), Set semantics (CE-138), Embed/remove unused fonts (CE-130, CE-420), External link check and download (CE-143/144; need real HTTP `LinkChecker`/`Fetcher` in the app crate), Check-book: help, per-error fix, skip, next/previous, accessibility toggle (CE-160..CE-165), Accessibility dialog (CE-139).
10. Smarten punctuation (CE-132) and remove unused CSS (CE-134): move/wrap the `bookx-convert` transforms into `bookx-editing` tools; add remove unused images (CE-135).
11. Compare dialog with text diffs (CE-224..CE-227) and checkpoint list dialog (CE-220/223).

**Wave 3 - navigation and completion (engine + UI)**
12. Ctrl+click, class rename, jump to definition (CE-342..CE-344): needs the Wave 1 scanner and a "definition index" (ids, classes -> rule) in `bookx-editing`.
13. Auto-complete (CE-345..CE-347), insert image/hyperlink (CE-354), quick open (CE-010) share one subsequence matcher.
14. Tabs (CE-011) and edit next/previous file (CE-009).
15. Source offsets in the document model, sync both ways (CE-243, CE-244), then pick-a-location mode (CE-246) -> ToC location (CE-124); split at cursor (CE-248; needs `SplitSpec::AtOffsets`), split dialog (CE-249).

**Wave 4 - scripting and snippets (new subsystem)**
16. Snippet expander (pure) + key handling (CE-380..CE-387).
17. Saved searches (CE-084) -> search ignoring HTML tags (CE-083) -> function mode with Rhai (CE-100..CE-113). Needs the Wave 1 regex decision and, for `dictionaries`, Wave 2 spelling.

**Wave 5 - niche**
18. Live CSS panel (CE-260; needs cascade API in `bookx-text`), preferences (CE-400..CE-403), special characters (CE-300..CE-302), HTML5-based Fix HTML (CE-136), CLI `--edit` (CE-018), the missing check classes of section 2.6 (CE-167..CE-209).

## 4. Keyboard shortcuts

Calibre default shortcuts in the editor (from `tweak_book/ui.py`, `editor/widget.py` and the manual; "in manual" marks the ones the manual page states) and what Bookx does. Since wave 5 every Bookx editor action is a registry entry with a default key (`src/editor_keymap.rs`, group *Editor*) that can be rebound in Preferences → Keyboard Shortcuts; where a default differs from Calibre: Ctrl+J insert snippet (Calibre: expand), Ctrl+Shift+F7 check spelling. Ctrl+Z/Ctrl+Shift+Z (undo/redo) come from `GtkTextView`.

| Calibre shortcut | Action | Bookx equivalent / gap |
|---|---|---|
| Ctrl+O | Open book | gap (CE-013) |
| Ctrl+S | Save | DONE |
| Ctrl+Alt+S / Ctrl+Alt+Shift+S | Save a copy / save a copy and edit it | gap (CE-005) |
| Ctrl+Q | Quit editor | window close (Ctrl+W/Alt+F4 by platform); gap as a binding |
| Ctrl+P | Preferences | Bookx: Ctrl+, opens the application Preferences with the *Editor* page; *Editor Preferences* in the palette; DONE (CE-400) |
| Ctrl+T | Quick open a file | gap (CE-010) |
| Ctrl+Alt+Down / Ctrl+Alt+Up | Edit next / previous file | gap (CE-009) |
| Ctrl+Left / Ctrl+Right | Revert to before / after (global undo/redo) | gap (CE-008) |
| Ctrl+Z / Ctrl+Y | Undo / redo typing | DONE through `GtkTextView` defaults (tooltips mention Ctrl+Shift+Z for redo) |
| Ctrl+X, Ctrl+C, Ctrl+V (also Ctrl+Insert, Shift+Insert) | Clipboard | DONE (GTK) |
| Ctrl+W / Ctrl+Alt+W / Ctrl+Shift+W | Close tab / other tabs / tabs to the right | gap (CE-011) |
| F1 | Manual | gap (CE-017) |
| F5 | Refresh preview | done (CE-242) |
| Ctrl+Alt+C | Copy from preview | done (CE-247) |
| Ctrl+Shift+R | Reports | DONE (CE-303) |
| Ctrl+F | Find/replace panel | DONE (also Ctrl+H) |
| F3 or Ctrl+G | Find next | F3 DONE; Ctrl+G gap |
| Shift+F3 or Shift+Ctrl+G | Find previous | Shift+F3 DONE; Shift+Ctrl+G gap |
| Ctrl+R | Replace | gap (button only) |
| Ctrl+] / Ctrl+[ | Replace and find next / previous | gap (CE-062/063) |
| Ctrl+A (in panel) | Replace all | gap (button only) |
| Ctrl+N (in panel) | Count all | gap (CE-065) |
| Ctrl+Shift+M | Mark selected text | DONE (CE-077, CE-425); Clear marks: palette |
| Ctrl+. | Go to line | gap (CE-080) |
| F7 / F8 / Shift+F8 | Check book / next / previous problem | DONE (Bookx uses F8 instead of Ctrl+F7) |
| Shift+F7 | Check grammar | gap, not recommended (CE-281) |
| F8 | Next spelling mistake | Bookx: Ctrl+F8 / Ctrl+Shift+F8, because F8 is next problem (CE-163, CE-280) |
| Shift+F8 / Ctrl+Shift+F8 | Next grammar issue / suggest rewrites | not recommended |
| Ctrl+M | Merge files | gap (CE-027) |
| F2 (file list) | Rename file | gap as key; button exists |
| Delete (file list) | Delete selected files | gap (CE-034) |
| Ctrl+Shift+Up/Down/Home/End (file list) | Move file in the spine | gap (CE-029) |
| Ctrl+B / Ctrl+I | Bold / italic | DONE (CE-350); Ctrl+U underline too |
| Ctrl+1 ... Ctrl+6, Ctrl+7 | Convert paragraph to h1-h6 / p | gap (CE-351) |
| Ctrl+` | Smart comment | gap (CE-349) |
| Ctrl+< / Ctrl+> / Ctrl+Alt+> | Insert / remove / split tag | DONE (CE-352), also Ctrl+Alt+W / Ctrl+Alt+U |
| Ctrl+{ / Ctrl+} (in manual) | Jump to opening / closing tag | gap (CE-335) |
| Ctrl+Alt+T or Ctrl+Shift+T (in manual) | Select tag contents | gap (CE-335) |
| Ctrl+J (in manual) | Expand snippet / next placeholder | Bookx: Tab expands and moves on; Ctrl+J opens *Insert Snippet* (CE-383) |
| Alt+X (in manual) | Convert typed hex code to character | DONE (CE-301) |
| Ctrl+click on file name or class (in manual) | Open file / jump to style rule | DONE (CE-342, CE-344); F12 does the same from the keyboard |
| Ctrl+click in special-character dialog | Insert and close | DONE, but Ctrl+click keeps the window open and a plain click closes it (owner brief) (CE-300); window on Ctrl+Alt+I |
| Tab (in completion popup) | Accept a completion | DONE (CE-345) |

## 5. Features worth NOT copying (recommendations only; the decision is the owner's)

| Feature | Reason |
|---|---|
| Dockable, nestable windows, per-file-type customisable toolbars, toolbar icon size (CE-016, CE-404) | Conflicts with libadwaita's adaptive, convention-driven layout; the E-ink and small-window modes of Bookx need a fixed responsive layout |
| Code inspector (CE-261) | Needs a browser engine; Bookx previews with its own `BookCanvas`. A Live CSS panel (CE-260) gives the same insight without fake DevTools |
| Python function mode (CE-100) | Roadmap and Part 2 say Rhai; a Python dependency is excluded by "Rust only". Port the API shape (match, number, file_name, metadata, data, `call_after_last_match`, `file_order`) and the recipes, not the language |
| Grammar checking with neural networks and GPU options (CE-281) | Heavy, English-centric, large model downloads; poor fit for an offline small reader. Spell checking with Hunspell is enough |
| Compare ORIGINAL_FMT to FMT, Book-details compare (CE-228/229) | Bookx has no `ORIGINAL_` format files; checkpoints and the library backup cover the use case |
| Separate Checkpoints and global undo/redo and a separate "Revert to" command set | Bookx's checkpoint stack is already the single undo for multi-file work; keep one concept (CE-008) |
| Copy files between editor instances (CE-038) and "Open with" external programs (CE-042) | Niche; export/replace (CE-035/036) already cover the round trip |
| Bookmarks file check, Nook cover quirk, ADE CMYK rules (CE-193, CE-199, CE-183) | Calibre/legacy-device specific; add only when a Bookx device profile needs them |
| "Update metadata embedded in the book when opening" (CE-405) | Bookx keeps OPF and database in sync in one place |
| Preferred-format order, integration-with-calibre preferences (CE-400 subset) | Not applicable |
| `ebook-edit --detach` | Linux-launcher detail; a plain `--edit` command is enough (CE-018) |
| Replacing the checkpoint model by a Calibre-style full-diff-on-save | Bookx already has deduplicated checkpoints (`Checkpoints`); keep them |
| Adopting a heavy editor widget wholesale | Option to consider, not copy: `GtkSourceView 5` would provide highlighting, bracket matching, snippets (`$1`, `${1:default}`, mirroring) and completion providers in one dependency. The pure-Rust scanner approach (`syntax.rs`, `markup::scan`) keeps logic unit-testable; decide before Wave 1 because it changes how CE-334, CE-345, CE-380 are built |

## Wave 4 notes: search and replace (editor-search)

- Adding `rhai` (which implements `Add<ImmutableString> for String`) made `String + &String` ambiguous in the `bookx` crate; two sites in `src/ui/library.rs` were changed to `.as_str()`. Other agents adding `a + &b` with `String` operands will hit the same error.
- Function mode: limits are 500,000 operations per call, 30 s per run, 4 MB strings, 100k array items, 10k map entries; a failed run changes nothing (the plan is applied only after every file ran). `data` is a shared store (indexer), `data["k"].push(x)` works.
- Ignore-markup and tag modes cannot be combined with the scope "Marked text"; saved searches cannot run "Marked text".
- Highlight in tag/ignore-markup mode is an own text tag set when the search changes or the file switches (not live while typing in the buffer).
- Done in wave 5: fuzzy mode (CE-424), search in the preview (CE-086), `functions` argument (CE-107), `append_final_output_to_marked` (CE-110). Not done: per-file summary dialog of Replace All (CE-064), the full special-character dialog inside the search fields (another agent's `special_chars.rs` exists; mine is a small popover).
- Tests: unit tests in `search_ignore`, `tag_search`, `editor_search`, `editor_search_fn` (limits: endless loop, time, string/array growth, recursion, no I/O), `editor_search_modes`, `editor_search_saved`; UI scenarios `editor_search_bar`, `editor_search_markup_tag`, `editor_saved_searches`, `editor_function_mode`; real session `editor_search_bar_session`.
