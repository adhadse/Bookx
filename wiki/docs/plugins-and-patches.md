# Plugins and patches

Bookx can be extended in two ways. Both are **off until you switch them on** and both are **sandboxed**.

| | Plugins | Patches |
|---|---|---|
| What | WebAssembly modules, written in any language | small TOML files with regex and CSS rules |
| Adds | metadata sources, file readers, output formats, transforms, device drivers, editor actions | text and style fixes at fixed moments |
| Folder | `~/.local/share/bookx/plugins/<name>/` | `~/.local/share/bookx/patches/*.toml` |

The folders can also be set with the `BOOKX_PLUGINS` and `BOOKX_PATCHES` environment variables.

## Using plugins

1. Put the plugin folder (with `plugin.toml` and `plugin.wasm`) into the plugins folder. *Preferences → Extensions →
   Open plugins folder* opens it.
2. Open *Preferences → Extensions* and switch the plugin on.

What a plugin adds shows up in the app:

| Kind | Where you see it |
|---|---|
| metadata source | *Fetch Metadata* in the edit dialog |
| file reader | adding books |
| output format | library → *Convert To*, entry "name (.ext, plugin)", and *Send to* |
| transform | library → *Convert To* → "Apply name (new EPUB)", the editor's Tools menu, and optionally every conversion |
| device driver | *Send to \<name\>* |
| editor action | the editor's *Tools → Plugins* menu and the command palette |

A transform can be added to every conversion with the switch "Run … in every conversion" in Preferences. It then
runs after Bookx's own steps in *Convert To* and *Send to*.

### Permissions

Preferences shows what every plugin may do, for example "Permissions: none. It cannot read your files or reach the
network." or "may connect to openlibrary.org". A plugin gets only what its manifest asks for, and it has no file
access at all: for a device driver, Bookx does the reading and writing. A plugin with a broken file is listed with
its error and cannot be switched on.

An **editor action** plugin can read and change the book you have open in the editor, nothing else. Preferences says
so: "may read and change the book open in the editor" (permission `editor.write`) or "may read the book open in the
editor" (`editor.read`). Every action first takes a checkpoint called "Before: \<action\>", all its changes are one
step, and *Tools → Restore Checkpoint* undoes them; an action that fails leaves the book as it was. A plugin cannot
touch `mimetype` or `META-INF/`, other files on your computer or the network. The actions appear in *Tools → Plugins* and
in the command palette (Ctrl+Shift+P); a plugin may suggest a keyboard shortcut, but Bookx never binds one by itself.

Plugins run in an interpreter with limits on memory, instructions and time. A crashing or misbehaving plugin
becomes an error message in the app, never a crash.

## Using patches

A patch is one `.toml` file. It applies regex or CSS rules to the book's text files at a chosen moment:

| Moment | Effect |
|---|---|
| `import-after` | after a book is converted to EPUB for the library; the stored book changes |
| `convert-before` | when you convert or send, before Bookx's own steps; the library copy does not change |
| `convert-after` | the same, after Bookx's steps and plugin transforms |
| `open-before` | when the reader opens a book; only what you see changes |

Manage patches in *Preferences → Patches*: a switch per patch (off by default), buttons to apply a patch
earlier or later, and a **dry run** that shows how many changes it would make in your first library book (or a
built-in sample) with a before → after excerpt, without changing anything.

Patches only edit `.xhtml`, `.html`, `.htm`, `.css`, `.svg`, `.txt`, `.ncx` and `.xml` files, never `mimetype`,
`META-INF/` or the package file, and cannot start programs or use the network. An invalid patch is listed with the
reason and cannot be switched on.

## Writing your own

See the [plugin and patch developer guide](plugin-development.md).

!!! note "Limitations"
    - There is no plugin store or browser. Plugins are folders you place yourself.
    - API versions 1 and 2 exist (2 adds editor actions). File-system permissions (`fs:*`) are never granted.
    - Plugin and patch support is tested with sample plugins built for the tests; few real-world plugins exist yet.
