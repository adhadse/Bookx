# Plugin and patch developer guide

Bookx can be extended in two ways, both **off until the user switches them on** (Preferences → Extensions; see [Plugins and patches](plugins-and-patches.md))
and both **sandboxed**:

| | Plugins | Patches |
|---|---|---|
| What | WebAssembly modules (any language) | small TOML files with regex / CSS rules |
| Adds | metadata sources, file readers, output formats, transforms, device drivers, editor actions | text and style fixes at fixed moments |
| Where | `~/.local/share/bookx/plugins/<folder>/` (`$BOOKX_PLUGINS`) | `~/.local/share/bookx/patches/*.toml` (`$BOOKX_PATCHES`) |
| Code | `crates/bookx-plugin` (`adapters`, `manager`, `runtime`) | `crates/bookx-plugin/src/patch.rs` |

Preferences shows what every plugin may do ("Permissions: none. It cannot read your files or reach the
network." or "may connect to openlibrary.org"). Nothing runs before the switch is on, and a broken file is
listed with its error instead of being loaded.

---

## 1. Plugins

### Layout

```
my-plugin/
  plugin.toml   # manifest
  plugin.wasm   # the module named by `wasm` (default plugin.wasm)
```

```toml
id = "org.example.rawcopy"        # lower case letters, digits . _ -  (max 64)
name = "EPUB copy"
version = "1.0.0"                 # digits and dots
api_version = 1                   # the ABI below
kind = "format-writer"            # metadata-source | format-reader | transform | format-writer | device | editor-action
description = "…"
permissions = []                  # ["network:openlibrary.org"]; fs:* is never granted in v1
extension  = "epubcopy"           # format-writer: extension of the file written
extensions = ["fb3"]              # format-reader: extensions it reads; device: formats it opens, best first
memory_pages = 256                # optional limits: 64 KiB pages, fuel (instructions), timeout in ms
fuel = 100000000
timeout_ms = 5000
```

### ABI v1

The module runs in the pure-Rust `wasmi` interpreter: no WASI, no file system, no sockets, no clock except
what the host lends. Every call runs in a **fresh instance**, so no state survives between calls.

Exports the module must provide:

* `memory`: linear memory.
* `alloc(len: i32) -> i32` and `dealloc(ptr: i32, len: i32)`.
* `bookx_call(op_ptr, op_len, in_ptr, in_len: i32) -> i64`. The operation name and a UTF-8 JSON input are
  in memory; return `(reply_ptr << 32) | reply_len` pointing at a JSON reply, or a negative code: `-1`
  failure, `-2` unsupported operation, `-3` invalid input, `-4` out of memory.

The reply is `{"ok": <value>}` or `{"err": "message"}`.

Host imports (module `bookx_host`; any other import makes the plugin refuse to load):
`log(level, ptr, len)`, `http_get(url_ptr, url_len, out) -> i32` (only hosts named in a `network:`
permission), `now_ms() -> i64`. Full details and error codes: the crate documentation of `bookx-plugin`.

Limits: memory is capped at `memory_pages`, instructions at `fuel` (the timeout is also turned into fuel),
replies at 16 MiB, 16 HTTP requests per call. Traps and garbage replies are errors in the app, never crashes.

### Editor plugins (ABI v2)

An `editor-action` plugin acts on the book that is open in the editor. It needs `api_version = 2`, at least one
`[[action]]` and the permissions it uses; everything else (limits, layout, ABI v1 exports) is as above, and v1 plugins
are not affected.

```toml
id = "org.example.trim"
name = "Trim trailing whitespace"
version = "1.0.0"
api_version = 2
kind = "editor-action"
permissions = ["editor.read", "editor.write"]   # editor.read: look at the book; editor.write: change it

[[action]]
id = "trim"                        # passed to the plugin as {"action": "trim"}
label = "Trim Trailing Whitespace" # the menu entry (max 80 characters)
group = "Cleanup"                  # optional
shortcut = "<Ctrl><Alt>t"          # optional suggestion, never bound by itself
```

The plugin gets one more export of the usual kind: `bookx_call("run_action", {"action": "<id>"})`, reply `{"ok": null}`
or `{"ok": "message shown to the user"}` or `{"err": "…"}`. And one more host import, `bookx_host::editor_call(op_ptr,
op_len, in_ptr, in_len, out) -> i32`: the operation name and a JSON object go in; the host allocates the reply in the
plugin through its `alloc` and writes `ptr` and `len` (two little-endian `u32`) at `out`. The reply is `{"ok": …}` or
`{"err": "…"}`. Return codes: `0` ok, `-1` bad arguments or unknown operation, `-2` permission not granted, `-3` limit
reached, `-4` the operation failed, `-5` the plugin could not take the reply, `-6` called outside an action, `-7`
timed out.

| operation | needs | input | `ok` |
|---|---|---|---|
| `list_files` | `editor.read` | `{}` | `["OEBPS/ch1.xhtml", …]` |
| `read_file` | `editor.read` | `{"path"}` | `{"text"}` or, for binary files, `{"b64"}` |
| `current_file` | `editor.read` | `{}` | path or `null` |
| `get_selection` | `editor.read` | `{}` | `{"file","text"}` or `null` |
| `get_metadata` | `editor.read` | `{}` | title, authors, language, … |
| `write_file` | `editor.write` | `{"path","text"}` or `{"path","b64"}` | the file must exist |
| `add_file` | `editor.write` | same | the file must not exist |
| `rename_file` | `editor.write` | `{"from","to"}` | links to the file are updated; `{"references_updated": n}` |
| `replace_selection` | `editor.write` | `{"text"}` | replaces the selected text of the open file |
| `insert_at_cursor` | `editor.write` | `{"text"}` | |
| `set_metadata` | `editor.write` | the fields to change | |
| `add_checkpoint` | `editor.write` | `{"name"}` | an extra named checkpoint |
| `show_message` | nothing | `{"text"}` | shown in the editor |

Rules the host enforces, whatever the plugin does: paths are inside the book (no absolute paths, no `..`, no
backslashes); `mimetype` and `META-INF/` are read-only; a file is at most 16 MiB; at most 5000 host calls, 2000
writes, 100 new files and 20 messages per action, and the usual fuel, memory and time limits. There is no file system,
no network and no clock beyond `now_ms`. Before the first change Bookx takes the checkpoint "Before: \<action label\>"; a
plugin that fails changes nothing, and *Tools → Restore Checkpoint* undoes one that succeeded.

A complete example in Rust (no dependencies, about 150 lines) is `examples/plugins/trim-whitespace/`. Build it with
`cargo build --release --target wasm32-unknown-unknown` and copy the `.wasm` as `plugin.wasm` next to `plugin.toml`
(a prebuilt `plugin.wasm` is included).

### Compatibility and the ABI stability policy

Plugins are compiled once and then live for years, so the host treats the ABI like a public API:

* **A plugin keeps working.** A plugin built for `api_version = 1` or `2` loads and behaves the same in every later
  Bookx 0.x and 1.x release. The host never removes an export, an import, an operation, a return code or a field of
  the JSON replies that a released ABI version defined.
* **Changes are additive.** New operations, new optional JSON fields, new manifest keys and new host imports may appear
  in a release; a plugin must ignore JSON fields it does not know, and the host does the same for fields a plugin
  adds. A new capability that old plugins could not know about needs a new `api_version` (that is how editor
  actions arrived: ABI v2, with `api_version = 1` plugins unaffected).
* **Versions are numbers, not releases.** A plugin states the `api_version` it was written for. The host lists the
  versions it speaks (today 1 and 2) and refuses others with a message that names both. A plugin never needs to
  mention the Bookx release.
* **Breaking changes** (they should be rare and are never made in a patch release) get a new `api_version`, keep
  loading the older one for at least two minor releases, and are announced in `CHANGELOG.md`. The limits (memory,
  fuel, time, reply size, call budgets) may only get *more* generous; a permission is never silently widened for an
  existing plugin, and a permission that a new ABI adds is not granted to older plugins.
* **How this is enforced.** `crates/bookx-plugin/tests/golden/` contains a v1 format writer and a v2 editor action
  as compiled binaries that are never rebuilt by the tests, and `tests/abi_compat.rs` loads and runs them. A host
  change that breaks one of them fails `cargo test`, and `build-aux/release-check.sh` fails if a golden file was
  changed. Updating a golden file is only legitimate together with a new ABI version.

### Operations per kind

Books travel as base64 of the EPUB (zip). Metadata objects use the field names of `bookx_core::opf::Metadata`
(`title, authors, publisher, language, description, tags, series, series_index, identifiers, published,
rights, rating`), all optional.

| kind | op | input | `ok` value | Where it shows up |
|---|---|---|---|---|
| metadata-source | `lookup` | `{"title","authors":[],"isbn"}` | array of metadata | *Fetch Metadata* |
| metadata-source | `cover` | a metadata object | `null` or base64 image | *Fetch Metadata* |
| format-reader | `sniff` | `{"head_b64"}` (first 4 KiB) | bool | adding books |
| format-reader | `read` | `{"data_b64"}` | base64 EPUB | adding books |
| **format-writer** | `write` | `{"book_b64"}` | base64 of the output file | library → *Convert To* (“<name> (.ext, plugin)”) and *Send to* |
| **transform** | `apply` | `{"book_b64"}` | `{"book_b64"?,"files_changed","notes":[]}` | library → *Convert To* → “Apply <name> (new EPUB)”, editor → Tools, and optionally every conversion |
| **device** | `detect` | `{"mounts":[{"path","name","entries":[…]}]}` | `[{"id","name","mount"}]` | *Send to <name>* |
| **device** | `send` | `{"device":{id,name,mount},"file_name","data_b64"}` | `{"path":"rel/file","data_b64"?}` | |
| **device** | `list` | `{"device":{…},"files":["rel",…]}` | array of relative paths | |

How the UI uses them:

* **Writers.** An enabled writer adds an entry to *Convert To*; the result is saved like any other format.
  Device plugins and the device window can use writer formats too.
* **Transforms.** An enabled transform is offered in the library (*Convert To → Apply … (new EPUB)* saves a
  transformed copy) and in the editor (*Tools → Plugins → …*, undoable like every editor tool). In Preferences
  a switch “Run … in every conversion” adds it, after the built-in steps, to Convert To and Send to. When it
  returns `book_b64` that replaces the book, otherwise it only reports.
* **Devices.** The plugin has no file system. For `detect` the host lists the top-level names of every
  candidate mount point (the sub-folders of the device roots) and the plugin answers with the devices it
  recognises; `mount` must be one of the offered paths. `send` names the destination **relative to the mount
  point** and the host does the writing; paths with `..`, absolute paths and empty paths are refused.
  `extensions` in the manifest are the formats the device opens, best first. Device ids are prefixed with the
  plugin id (`org.example.dev:r1`).

### Worked example: a format-writer in WebAssembly text

[`examples/plugins/rawcopy-writer/`](https://github.com/adhadse/Bookx/tree/main/examples/plugins/rawcopy-writer) holds a complete plugin: `plugin.toml` and `plugin.wat`. The writer
answers `write` with the same EPUB, saved as `.epubcopy`. It needs no JSON library because the input
`{"book_b64":"…"}` and the output `{"ok":"…"}` differ only in a prefix. The test
`crates/bookx-plugin/tests/example_plugin.rs` assembles it, enables it through the manager and checks the
output, including a 3 MiB book (memory growth) and bad input.

Try it by hand:

```bash
wat2wasm examples/plugins/rawcopy-writer/plugin.wat -o plugin.wasm     # or: cargo test -p bookx-plugin --test example_plugin
mkdir -p ~/.local/share/bookx/plugins/rawcopy
cp examples/plugins/rawcopy-writer/plugin.toml plugin.wasm ~/.local/share/bookx/plugins/rawcopy/
# main menu → Preferences → Extensions → switch it on; the library's Convert To menu now offers “EPUB copy (example writer)”
```

### Writing a plugin in Rust

Build a `cdylib` for `wasm32-unknown-unknown` that exports `alloc`, `dealloc` and `bookx_call` as above; the
crate documentation of `bookx-plugin` has a complete skeleton. The repository's integration fixture
`crates/bookx-plugin/tests/fixtures/rust_plugin` is a real one.

### Testing a plugin

* Unit-level: load it with `bookx_plugin::Plugin::from_bytes(manifest, wasm, Host::default())` and call
  `call_json("op", &json)`; wrap it with the adapters (`PluginWriter`, `PluginTransform`, `PluginDevice`, …) to
  use it through the `bookx_core::contracts` traits.
* The test helpers in `crates/bookx-plugin/tests/common` assemble mock plugins from WAT in a few lines
  (`canned`, `echo`, `always`).
* For the app: `cargo run -p bookx-plugin --example make_test_plugins -- <plugins dir> <mounts dir>` writes
  one plugin of each new kind; `build-aux/ui-tests.sh` scenarios `plugin_writer`, `plugin_transform` and
  `plugin_device` drive them through the `plugins:*` dev commands.

---

## 2. Patches

A patch is one `.toml` file in the patches folder. It is applied at a declared **hook** to the EPUB inside
Bookx; it cannot touch any other file, start programs or use the network. Regular expressions use the Rust
`regex` engine (linear time), with a size limit; every limit is enforced when the file is read.

```toml
id = "straight-quotes"            # optional, default: the file name without .toml
name = "Straight quotes"
description = "Replaces curly double quotes by plain ones"
hook = "convert-before"           # import-after | convert-before | convert-after | open-before
files = ["*.xhtml", "*.html"]     # optional default for the rules; * and ? globs on book paths

[[regex]]
find = '[“”]'                     # Rust regex syntax
replace = '"'                     # $1 / ${name} are groups, $$ is a dollar sign
flags = "i"                       # optional: i (ignore case), s (dot matches newline), m (multi-line)
files = ["OEBPS/*.xhtml"]         # optional, overrides the default

[[css]]
selector = "p"                    # appended to each matching style sheet as `p { text-indent: 0 }`
declarations = "text-indent: 0"
files = ["*.css"]                 # optional, default *.css
```

| hook | when | effect |
|---|---|---|
| `import-after` | a book was converted to EPUB for the library | the stored book changes; metadata and cover are read again |
| `convert-before` | *Convert To* / *Send to*, before Bookx's own steps | the converted file changes, the library copy does not |
| `convert-after` | same, after Bookx's own steps and plugin transforms, before writing | |
| `open-before` | the reader opens a book | only what you see changes; the file stays as it is |

Rules run in the file order, patches in the order shown in Preferences. A glob without `/` matches the file
name; with `/` the whole path. Matching ignores case.

Safety rules: only `.xhtml .html .htm .css .svg .txt .ncx .xml` files are edited, never `mimetype`,
`META-INF/` or the package (`.opf`). Limits: 64 KiB per patch file, 100 rules, patterns of at most 2000
characters, files up to 8 MiB, 100 000 replacements per file. Invalid files (unknown keys, bad regex, braces
in CSS, unknown hook) are listed on the Extensions page of Preferences with the reason and cannot be switched on.

### Managing patches

The **Patches** group on the Extensions page of Preferences (main menu → Preferences) lists the files in application order. Per patch: a switch (off by
default), buttons to apply it earlier or later, and a **dry run**: it shows how many changes the patch would
make in your first library book (or a built-in sample) and a before → after excerpt per file, without changing
anything. The switch state and the order are kept in `patches.json` next to the other settings.

!!! note "Limitations"
    - Only plugin API version 1 exists.
    - Plugins have no file-system access and no sockets other than the hosts named in `network:` permissions.
    - The Rust skeleton referred to above lives in the `bookx-plugin` crate documentation (`cargo doc -p bookx-plugin`).
