# Bookx editor: UI design (inspired by GNOME Builder)

Status: design note. The implemented subset is listed in section 8; the rest is the target the
gap analysis (`docs/calibre-editor-gap-analysis.md`) is mapped onto.

**Sources.** GNOME Builder's public user documentation and its screen layout, as known from its
manual and from using it (no web access was available while writing this note, so the Builder
repository pages were not re-read). Only ideas are taken: layout, panel names, interaction
patterns. No Builder code (GPL) is copied or ported, and the Bookx implementation is
GTK4/libadwaita/Relm4 code written for this repository.

## 1. Layout

```
+--------------------------------------------------------------------------------------+
| [Save][Undo][Redo]        Book title  ·  OEBPS/ch01.xhtml     [Find][Check][Tools][Pv]|  header bar
+---------------+----------------------------------------------------------------------+
| Files|Outline |  [ch01.xhtml x][style.css x][cover.jpg x]                  (tab bar)  |
| |Search|Meta  +------------------------------------------+---------------------------+
|               |  Find [______] [.*][Aa][.⏎] [This file v] [^][v] [Find All]            |  search bar
| Text          |  Replace [____] [Replace][Replace All]      3 of 12                   |
|  1 ch01.xhtml +------------------------------------------+---------------------------+
|  2 ch02.xhtml |  1 | <?xml version="1.0"?>               |  Preview (follows cursor) |
| Styles        |  2 |▌<html xmlns=...>      (gutter mark)  |                           |
|  style.css    |  3 |  <body>                              |   Chapter One             |
| Images        |  4 |    <h1>Chapter One</h1>              |   Lorem ipsum ...         |
|  cover.jpg    |  5 |    <p>Teh quick ...                  |                           |
| (multi-select)|    |           ~~~ spelling underline     |                           |
+---------------+----------------------------------------------------------------------+
| Check results | Reports | Search results                                         [x]   |  bottom panel
|  ⚠ OEBPS/ch01.xhtml:5  RSC-007 Broken link ...        [Fix] [Skip kind]              |
+--------------------------------------------------------------------------------------+
| ch01.xhtml · Ln 5, Col 12 · XHTML · en-US · 3 spelling · Checkpoint 4 min ago        |  status bar
+--------------------------------------------------------------------------------------+
   Ctrl+P quick open (popover over the window)      Ctrl+Shift+P command palette
```

## 2. What is borrowed from Builder, what is not

| Builder idea | Bookx |
|---|---|
| Tabbed editor pages, one buffer per page | Borrowed (CE-009..CE-011, implemented: `adw::TabBar` over the per-file buffers) |
| Left sidebar with switchable panels (project tree, outline, search) | Borrowed: Files, Outline (the ToC editor), Search (results), Metadata |
| Bottom panel for diagnostics/build output | Borrowed: the results revealer becomes the bottom panel with Check results, Reports, Search results |
| Global search / "Ctrl+P"-style quick open, command palette | Ctrl+P quick open is implemented (subsequence matching); the Ctrl+Shift+P action list is only designed (section 6) |
| Search bar with replace and live match count | Borrowed: the bar exists; the count ("N of M") is live |
| Diagnostics gutter marks, inline squiggles | Borrowed in design: gutter marks for "changed since checkpoint" and Check findings, underline for spelling |
| Status bar with line/column, language, encoding | Borrowed (line/column, file, language) |
| Preview that follows the cursor | Borrowed in design (needs source offsets, CE-243/244) |
| GtkSourceView as editing widget | Not adopted (section 4) |
| Build/run/debug/terminal/VCS panels, project plugins, LSP | Not borrowed: out of scope for a book editor |
| Dockable, user-arranged panels | Not borrowed (CE-016): Bookx keeps a fixed adaptive layout |

## 3. Map of Calibre features (gap analysis ids) to places in the UI

**Header bar / menus**

* Save, Undo/Redo: CE-004, CE-359, CE-008 (global checkpoint undo as a second pair of buttons).
* Find toggle, Check (F7), Tools menu, Preview toggle: CE-060, CE-160, CE-240.
* Tools menu: CE-045, CE-127, CE-131, CE-132, CE-133, CE-134, CE-135, CE-136, CE-137, CE-138, CE-139,
  CE-140, CE-141, CE-142, CE-143, CE-144, CE-145, CE-146, CE-130, CE-129; checkpoints CE-220..CE-223;
  Reports CE-303..CE-311; Spelling CE-270..CE-279; Split/Merge CE-027, CE-248, CE-249.
* Primary menu: Save a copy CE-005, preferences CE-400..CE-403, shortcuts window CE-403, help CE-017.
* Markup menu (implemented, section 7) for select/jump to matching tag, close tag, wrap, remove surrounding
  tag, rename tag, change block to heading, toggle comment, change case: CE-334, CE-335, CE-349, CE-351..CE-353.
  Not yet: CE-332 (CSS inside `<style>`), CE-338 (hard-to-see characters), CE-348 (auto-close on `</`),
  insert image/link CE-354, special characters CE-300..CE-302.

**Tab bar and editor**

* Tabs, close others, next/previous file: CE-009, CE-011, CE-015. Scope "Open files" CE-076.
* Code view: highlighting CE-330..CE-333, gutter marks CE-336, CE-338, CE-339, line numbers CE-340,
  Ctrl+click and jump to definition CE-342..CE-344, completion popovers CE-345..CE-347, snippets
  CE-380..CE-387, spelling underline CE-280/337.

**Search bar** (revealer above the editor)

* CE-060..CE-070, CE-077..CE-081, CE-083..CE-085 (modes, scopes, regex dialect, saved searches,
  history). Function mode (CE-100..CE-113) is a fourth mode with an editor dialog.
* Go to line (Ctrl+L, popover) CE-080; Mark selected text CE-077.

**Sidebar**

* Files panel: CE-002, CE-020..CE-046 (groups, multi-select, rename/bulk rename, merge, delete,
  export, link stylesheets, new file dialog, context menu).
* Outline panel (Contents): CE-120..CE-126 (navigate on select, generate dialog, reorder),
  CE-260 Live CSS later as a fifth panel.
* Search panel: results of Find All (CE-082) and per-file counts (CE-065).
* Metadata panel: CE-428.

**Bottom panel**

* Check results: CE-160..CE-165 and every check class CE-166..CE-212, with Fix and Skip.
* Reports: CE-303..CE-311. Compare/diff view CE-224..CE-227 opens as a dialog from the checkpoint list.

**Preview pane**: CE-240..CE-247 (refresh, auto sync, click to jump, pick a location CE-246/124).

**Status bar**: line/column, file path (CE-021), language, spelling count (CE-270), last checkpoint.

**Quick open / command palette**: CE-010; every action also appears in the palette, which is the
discoverable replacement for Calibre's customisable toolbars (CE-404 not copied).

## 4. GtkSourceView

**Adopted 2026-11-12** (owner decision, `docs/enhancements.md` item 5). The original recommendation was "not
now" because the library was not installed on the dev machine and the logic would only have been testable
through widgets. Both reasons are gone: `libgtksourceview-5` is installed, and the decisions live in GTK-free
modules (`editor_complete`, `editor_snippets`, `editor_marks`, `editor_spell`, `ui/editor/codeview.rs` pure
functions) with unit tests, while the widget glue has UI scenarios. The GNOME 49 Flatpak runtime ships
GtkSourceView 5.18, so the manifest needs no extra module. What was built and what was left out:
`docs/editor-sourceview.md`.

## 5. Interaction rules

* Every edit of the whole book (Replace All, Contents generate, delete/move/add/rename/merge files, bulk
  rename, link stylesheets, new file, metadata, tools) first takes an automatic checkpoint named
  "Before: ...". A tool that changed nothing removes its checkpoint again; one that fails restores it.
* Destructive actions on several files ask once and name the count.
* All shortcuts are also reachable through a menu entry; shortcuts are plain `gtk::Shortcut`s.
* Menu text, shortcut and the pure function behind it are listed in section 7, so each is tested without a GUI.
* Search results, Check results and Reports share one results area (the future bottom panel); a click
  jumps to the file and line.

## 6. Panels and the future command palette

The command palette (Ctrl+Shift+P) is simply the list of every `gio::Action` of the groups `tools.*`, `edit.*`
and `files.*` with its label and accelerator, filtered by typing. That is why all actions are `gio` actions
with string targets (`tools.run::fix`, `edit.tag::wrap`) and no closure lives only in a button handler.
Quick open (Ctrl+P) ranks `file_list()` by subsequence match on the file name first, then on the path.

## 7. Markup and file actions (implemented)

| Menu entry | Shortcut | Function (`bookx_editing`) | CE |
|---|---|---|---|
| Select to Matching Tag | Ctrl+Alt+K | `tags::select_to_matching` | CE-334, CE-335 |
| Jump to Matching Tag | Ctrl+Alt+J | `tags::jump_to_matching` | CE-335 |
| Close the Open Tag | Ctrl+Alt+. | `tags::close_tag` | CE-352 (related) |
| Wrap Selection in Tag… | Ctrl+Alt+W | `tags::wrap_in_tag` | CE-352 |
| Remove Surrounding Tag | Ctrl+Alt+U | `tags::remove_surrounding` | CE-352 |
| Rename Tag… | Ctrl+Alt+R | `tags::rename_tag` | CE-351 |
| Change Block to (Paragraph, Heading 1-6) | Ctrl+0 … Ctrl+6 | `tags::change_block_to_heading` | CE-351 |
| Change Case | menu only | `tags::change_case` | CE-353 |
| New File… | Ctrl+N | `files::create_from_template` | CE-039 |
| Rename (F2) | F2 | `files::rename_files` over `markup::rename_paths` | CE-023 |
| Bulk Rename… | menu | `files::bulk_rename_plan` | CE-024 |
| Merge Selected… | menu | `split_merge::merge_files` (master chosen) | CE-027 |
| Link Stylesheets… | menu | `files::link_stylesheets` | CE-041 |
| Export Selected… | menu | `files::export_files` | CE-035, CE-037 |
| Delete Selected | Delete | `files::delete_files` | CE-034 |
| Generate Contents… | button | `toc_gen::generate` + `combine` | CE-121..CE-123 |
| Select a Contents entry | click | `toc_gen::anchor_offset` | CE-126 |
| Toggle Comment | Ctrl+/ | `tags::toggle_comment` | CE-349 |
| Next / previous file | Ctrl+Alt+Down / Up | `nav::step_file` | CE-009 |
| Quick open | Ctrl+P | `files::quick_open` | CE-010 |
| Close tab | Ctrl+W | `adw::TabView` | CE-011 |

The tag scanner (`tags::pairs`) tolerates broken markup: void elements and self-closing tags have no partner,
a close tag pops everything opened after its match, stray close tags are ignored.

## 8. Implemented so far

Implemented and covered by unit tests and UI scenarios (see the gap analysis for the row statuses):
the module split of the editor, search bar with scopes and the fancy-regex dialect, go to line and marked
text, rename through `rename_paths`, automatic checkpoints for every whole-book change, multi-select file
list with its menu and context menu, tag actions, Contents navigation and the generate dialog, new file dialog.

Tabs, next/previous file and quick open (CE-009..CE-011) are implemented too.

Not implemented: command palette, outline/search as separate sidebar
panels, gutter marks, spelling underlines, cursor-following preview. (Drag and drop in the file list and the
Contents was left out here at first; both exist now, see sections 9 and 10.)

## 9. Tools dialogs, file browser extras, Book menu (added)

Source: `src/ui/editor/{tools_ui,css_transform_ui,files_extra,window_features,ext_dev}.rs`, logic in
`src/editor_tools/{more,rules}.rs`, `src/editor_state.rs` and `crates/bookx-editing/src/book_tools.rs`.

* One dialog shape for every tool with options (`Editor::tool_dialog`): options, **Preview** (the tool runs on a copy
  of the archive; `editor_tools::preview_report` lists changed files with before/after lines), **Apply** (`run_op`: a
  checkpoint, one toast). The tools only supply their options and a function that turns them into an `Op`.
* Network tools (check links, download) run on a worker thread, paced by `bookx_sources::fetch::Throttle`, and report
  through a channel polled on the main loop. `BOOKX_EDITOR_NET_MOCK` replaces the network in tests.
* The file list is built in `files_extra.rs`; rows of spine files are drag sources and drop targets. The drop handler
  (`Editor::drop_row`) is also what the dev hook `x:drop` calls; the session scenario `editor_file_drag_reorder` drags
  with a real pointer.
* Revert to before/after keeps its redo points as checkpoints named `After: …`; see `editor_state::undo_target`.
* Remembered per book: `editor-state.json` (tabs, caret). Window preferences: `editor-window.json`. User templates:
  `editor-templates/`. Rule sets: `editor-css-rules/`.
* Caveats: font search runs on a worker thread with a progress label (files over 12 MB skipped); the HTML5 rebuild writes attributes in sorted order; `bookx --edit` has the scenario `editor_x_edit_cli` through the `edit-file:` dev hook. Tool dialogs are sized
  to the window (`fit_dialog`) so Preview and Apply stay visible at 900x600; real-session smoke scenarios
  `editor_tools_smoke_1280` / `_900` open every dialog through the command palette.

## 10. Structure tools: Contents drag and drop, cover, fonts, HTML rules, jacket, accessibility form, other formats

Source: `src/ui/editor/{struct_hub,toc_dnd,cover_dialog,fonts_manager,html_transform_ui,jacket,a11y_form,unpack_repack}.rs`,
glue in `src/editor_structure.rs`, logic in `bookx_editing::{toc_ops,cover,fonts_manage,transform_html,jacket,a11y_meta}`.
Roadmap rows CE-028, CE-120, CE-129, CE-420, CE-421, CE-423, CE-426, CE-427, CE-429.

* **Contents drag and drop.** Each row has a grip (`list-drag-handle-symbolic`) that is the `DragSource` (a drag on
  the title entry would fight with text selection); the row is the `DropTarget` for a `u32` (the index). The pointer's
  place on the row picks `toc_ops::DropPos` (top quarter before, bottom quarter after, middle into). The handler is
  `Editor::toc_drop`, also called by the dev hook `x:toc-drop=<from>|<target>|before|after|into`. Alt+arrows are a capture
  phase key controller on the list (the entries would otherwise take the keys) that emits the same `TocOp`s as the
  buttons; after a keyboard move the focus returns to the entry of the moved item (`State::toc_refocus`). The class
  names `toc-drop-before/after/into` are installed once as a display-wide CSS provider.
* **Merge by dragging.** In `files_extra::make_draggable` the middle 40% of a page row (`struct_hub::is_merge_zone`) is
  "merge": the row gets `toc-drop-into`, the drop sends `StructCmd::DropMerge` and `prompt_drop_merge` asks, then
  `Input::MergeFiles([target, dragged])` runs the existing merge on a checkpoint. `x:drop-merge=<from>|<onto>` asks,
  `x:drop-merge-accept` answers yes (`x:drop-merge-pending` shows what waits).
* **Dialogs.** Transform HTML and Book Jacket use `tool_dialog` (Preview = a dry run on a copy). Set Cover, Manage
  Fonts and Accessibility Metadata have their own layouts; the accessibility form validates on every change and
  Apply refuses errors. All have dev hooks (`x:cover=`, `x:fonts-manage=`, `x:html-rules=`, `x:html-ruleset=`,
  `x:jacket=`, `x:a11y-form=`, `x:foreign-open=`, `x:foreign-save=`; `x:win-size=WxH`, `x:dialog-close`), see
  `src/ui/editor/struct_hub.rs` and `build-aux/ui-tests-editor-struct.sh`.
* **Other formats (CE-429).** `State::origin` holds the file a book came from. `Editor::save` is intercepted when it is
  set: `prompt_save_foreign` offers Save as EPUB / Convert Back. A foreign session uses `book_id` 0 and never touches the
  library. Per-book editor preferences (spelling dictionaries, check panel) are kept under the key `0` for these.
* **Screenshots.** Broadway cannot draw an `AdwDialog` into the window snapshot (the child widget is not mapped for
  `WidgetPaintable`), so dialog screenshots come from the real-session harness
  (`build-aux/session/scenarios_editor_struct.py`: `editor_struct_dialogs_1280`, `_900`).
