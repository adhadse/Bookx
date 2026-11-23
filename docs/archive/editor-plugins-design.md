> Archived 2026-11-23: Implemented in wave 5; the guide is `plugin-development.md`. Living document: [plugin-development.md](../plugin-development.md).

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

* Plugin-defined dialogs or panels, a plugin store.
* Wave 11 done: running one action on every book (`bookx-cli plugin run DIR ACTION [QUERY] [--apply]`, `bookx_plugin::batch::run_on_archive`: a
  copy of the book as `MemBook`, only changed files come back; selection calls fail, metadata edits are not applied; dry run by default);
  the capability sheet (`src/ui/plugin_trust.rs`: Allow/Cancel before a plugin is switched on, trust stored in `plugins.json` as the
  SHA-256 fingerprint of `plugin.toml` + wasm, a changed plugin is not registered until allowed again; plugins enabled before trust existed
  keep running); signing (below).
* Signing (G10): `plugin.sig` next to `plugin.toml`: `bookx-plugin-signature 1`, `key: <64 hex>`, `signature: <128 hex>`, an ed25519 signature over
  `"bookx-plugin-v1\0" || sha256(plugin.toml) || sha256(wasm)`. **No ed25519 implementation is in the dependency tree**, so
  `UnsupportedVerifier` answers "cannot check" and a signed plugin shows as "signed, not verified"; the parser (strict, 1 KiB limit), the message,
  the `SignatureVerifier` trait and the status/warning plumbing are tested with a fake verifier. Adding `ed25519-dalek` (or `ed25519-compact`)
  and a trusted-key list is one `impl`. Policy kept: unsigned plugins are allowed, with a warning in the sheet.
* Binding the suggested shortcuts (needs the keymap editor of another track).
* The wasm host has no `now_ms`-free determinism guarantee beyond the existing limits.
