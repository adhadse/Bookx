# Editor plugins (CE-019): design

Status: design only. Not implemented.

**Why not now.** `bookx-plugin` ABI v1 passes bytes in and out of a sandboxed wasm module for stateless adapters
(metadata sources, file readers, output formats, transforms, device drivers). An editor plugin needs the opposite: to
read and change the *open* book, create a checkpoint around its change, and add its own menu entry. That is a new
host-call surface, not a small additive change to v1, and it must be decided together with the permission model.

## Proposed ABI v2 (editor hooks)

* Manifest: `kind = "editor-action"` with `[[action]] id, label, group, shortcut?`.
* Host calls (all synchronous, run on a copy of the book; the host writes the result back through the same
  `Editor::run_op` as the built-in tools, so every plugin action is one checkpoint "Before: <label>" and one toast):
  * `book_list() -> [path]`, `book_read(path) -> bytes`, `book_write(path, bytes)`, `book_delete(path)`,
    `book_rename(from, to)` (references follow, as `markup::rename_paths`);
  * `book_metadata() -> json`, `current_file() -> path?`, `selected_text() -> string?`;
  * `log(line)` shown in the results panel.
* No network, no file system, no clock (same sandbox as v1). Fuel limit per call.
* The action list is added to the Tools menu under "Plugins" and to the command palette; the existing transform
  plugins already appear there, so the UI side is the `ext.tool` action with a `plugin-action:<id>` target.

## Open questions

* Whether actions may also run on the *selection* (needs a text-range host call and undo grouping in the buffer).
* Preview of a plugin action: the generic dialog preview (`editor_tools::preview_report`) works unchanged, since it
  only needs an operation that can run on a copy of the book.
