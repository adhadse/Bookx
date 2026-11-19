# Editor plugins (CE-019)

Status: **implemented** (wave 5). User guide: `wiki/docs/plugins-and-patches.md`; developer guide:
`docs/plugin-development.md` ("Editor plugins (ABI v2)"); code: `crates/bookx-plugin/src/editor.rs` (the host calls,
permissions, limits), `manifest.rs` (kind, actions), `runtime.rs` (the `editor_call` import), `src/ui/editor/plugin_host.rs`
(the editor side), `examples/plugins/trim-whitespace` (an example in Rust).

## What was decided

* **ABI v2 is additive.** `api_version = 2` is accepted next to 1; `kind = "editor-action"` requires 2. Plugins of the
  other kinds, and their imports and exports, are unchanged. The one new import is `bookx_host::editor_call(op, input,
  out) -> i32` with JSON in and out (one door instead of a dozen imports: every call goes through the same permission,
  path, size and call-count checks in `EditorSession::dispatch`). It is refused at load time for other kinds.
* **Capabilities.** `editor.read` (list/read files, selection, current file, metadata) and `editor.write` (write, add,
  rename, selection edits, metadata, extra checkpoints). `show_message` needs nothing. The manifest lists them, the
  Preferences row words them ("may read and change the book open in the editor"), plugins stay switched off until the
  user opts in (D5). `network:` is refused for editor plugins, `fs:*` is never granted.
* **Safety.** Paths are book-relative and checked (no absolute, `..`, backslash, control characters, `:`); `mimetype` and
  `META-INF/` are read-only; a file is at most 16 MiB, an action may make 5000 host calls, 2000 writes, 100 new files, 20
  messages and read/write 128/64 MiB; on top of that the usual fuel, memory and time limits and a fresh instance per
  action. A write is always preceded by the automatic checkpoint "Before: <label>".
* **Where the changes land.** The action runs on a **copy** of the archive (`BookCopy`). Only when the plugin returns
  success does `Editor::run_op` replace the book with the copy, so the whole action is one checkpoint and *Restore
  Checkpoint* is the undo; an error or a trap leaves the book untouched (and removes the checkpoint). Extra
  checkpoints the plugin asks for are snapshots of the copy at that moment, created after the run.
* **Selection.** The editor hands over the selection and cursor as character offsets in the open file (buffers are
  flushed first). `replace_selection` / `insert_at_cursor` edit that text in the copy, so they are part of the same
  checkpoint rather than separate GTK undo steps. After `write_file` on the open file the offsets are stale and the
  selection calls fail with a message.
* **UI.** Tools → Plugins (one submenu) and the command palette (the palette walks the Tools menu). The shortcut of an
  action is only a suggestion shown in Preferences; nothing is bound automatically.
* **Preview** is not offered: a plugin action is not side-effect free in principle (it could be, since it works on a
  copy, but the generic dialog preview needs a result list that plugins do not produce yet).

## Not done

* Plugin-defined dialogs or panels, a plugin store, signing. Running one action on every book of the library.
* Binding the suggested shortcuts (needs the keymap editor of another track).
* The wasm host has no `now_ms`-free determinism guarantee beyond the existing limits.
