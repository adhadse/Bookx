# Plugins and patches

Bookx can be extended in two ways. Both are **off until you switch them on** and both are **sandboxed**.

| | Plugins | Patches |
|---|---|---|
| What | WebAssembly modules, written in any language | small TOML files with regex and CSS rules |
| Adds | metadata sources, file readers, output formats, transforms, device drivers, editor actions | text and style fixes at fixed moments |
| Folder | `~/.local/share/bookx/plugins/<name>/` | `~/.local/share/bookx/patches/*.toml` |

The folders can also be set with the `BOOKX_PLUGINS` and `BOOKX_PATCHES` environment variables.

## Using plugins

<figure markdown="1">
![Preferences, Extensions](assets/preferences-extensions-light.png#only-light)
![Preferences, Extensions](assets/preferences-extensions-dark.png#only-dark)
<figcaption markdown="1">Preferences → Extensions: installed plugins and the folder they go into.</figcaption>
</figure>


1. Put the plugin folder (with `plugin.toml` and `plugin.wasm`) into the plugins folder. Open the main menu →
   **Preferences**, choose the **Extensions** page and press **Open plugins folder** to get there.
2. Back on the Extensions page, switch the plugin on. Bookx first shows what the plugin may do ("Read the book that is open
   in the editor", "Connect to example.org") and whether it is signed. Press **Allow** to switch it on. Plugins without a
   signature are allowed, with a warning. Bookx remembers your choice together with a fingerprint of the plugin files:
   if the files change later, the plugin stops running until you switch it off and on again and allow it again.

What a plugin adds shows up in the app:

| Kind | Where you see it |
|---|---|
| metadata source | *Fetch Metadata* in the edit dialog |
| file reader | adding books |
| output format | library → *Convert To*, entry "name (.ext, plugin)", and *Send to* |
| transform | library → *Convert To* → "Apply name (new EPUB)", the editor's Tools menu, and optionally every conversion |
| device driver | *Send to \<name\>* |
| editor action | the editor's *Tools → Plugins* menu and the command palette |

A transform can be added to every conversion with the switch "Run … in every conversion" on its row on the Extensions
page of Preferences. It then runs after Bookx's own steps in *Convert To* and *Send to*.

### Permissions

The Extensions page shows, under each plugin, what it may do, for example "Permissions: none. It cannot read your files or reach the
network." or "may connect to openlibrary.org". A plugin gets only what its manifest asks for, and it has no file
access at all: for a device driver, Bookx does the reading and writing. A plugin with a broken file is listed with
its error and cannot be switched on.

An **editor action** plugin can read and change the book you have open in the editor, nothing else. The Extensions page says
so: "may read and change the book open in the editor" (permission `editor.write`) or "may read the book open in the
editor" (`editor.read`). Every action first takes a checkpoint called "Before: \<action\>", all its changes are one
step, and *Tools → Restore Checkpoint* undoes them; an action that fails leaves the book as it was. A plugin cannot
touch `mimetype` or `META-INF/`, other files on your computer or the network. The actions appear in *Tools → Plugins* and
in the command palette (Ctrl+Shift+P); a plugin may suggest a keyboard shortcut, but Bookx never binds one by itself.

To run an editor action on many books at once, use `bookx-cli plugin run` (see [Command line](command-line.md)).

Plugins run in an interpreter with limits on memory, instructions and time. A crashing or misbehaving plugin
becomes an error message in the app, never a crash.

## Using patches

<figure markdown="1">
![Preferences, Patches](assets/preferences-patches-light.png#only-light)
![Preferences, Patches](assets/preferences-patches-dark.png#only-dark)
<figcaption markdown="1">Preferences → Extensions: installed patches and their folder.</figcaption>
</figure>


A patch is one `.toml` file. It applies regex or CSS rules to the book's text files at a chosen moment:

| Moment | Effect |
|---|---|
| `import-after` | after a book is converted to EPUB for the library; the stored book changes |
| `convert-before` | when you convert or send, before Bookx's own steps; the library copy does not change |
| `convert-after` | the same, after Bookx's steps and plugin transforms |
| `open-before` | when the reader opens a book; only what you see changes |

Manage patches in the **Patches** group lower on the same Extensions page (main menu → Preferences → Extensions): a switch per patch (off by default), a **⋯** menu with *Apply Earlier*, *Apply Later* and *Dry Run on Your First Book*, which shows how many changes it would make in your first library book (or a
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

## Will my plugins keep working?

Yes: a plugin declares the plugin interface version it was written for (`api_version`, today 1 or 2) and every later
Bookx release keeps running it unchanged. New abilities arrive as additions or as a new interface version, and a
release never takes a permission or an operation away from a released version. Plugin authors can read the details in
the [stability policy](plugin-development.md#compatibility-and-the-abi-stability-policy).

## Questions and answers

??? question "How do I install a plugin?"
    Open the main menu → **Preferences** and choose the **Extensions** page. Press **Open plugins folder**, copy the plugin's
    folder (the one with `plugin.toml` and `plugin.wasm`) into it, and come back: the plugin is listed. Switch it on.
    There is no plugin store; plugins are folders you place yourself.

??? question "Why is my plugin listed but cannot be switched on?"
    A plugin with a broken file is shown with its error and cannot be enabled. Read the message under its name; usually
    the manifest or the `.wasm` file is wrong or the plugin asks for a newer interface than your Bookx knows.

??? question "What can a plugin do to my computer?"
    Only what its manifest asks for, and you see it under the plugin on the Extensions page, for example “Permissions: none.
    It cannot read your files or reach the network.” or “may connect to openlibrary.org”. No plugin gets file access.
    Plugins run in a sandbox with limits on memory and time, and a misbehaving one becomes an error message, not a
    crash.

??? question "Where do I see what a plugin added?"
    A metadata source appears in **Fetch Metadata** in the book edit dialog; a file reader when you add books; an output
    format and a transform in the book's **Convert To** menu (and *Send to*); a device driver as *Send to name*; an editor
    action in the editor's **Tools → Plugins** menu and the command palette (++ctrl+shift+p++).

??? question "How do I run a transform on every conversion?"
    On the Extensions page, a plugin with a transform has a switch “Run … in every conversion”. It then runs after Bookx's own
    steps in **Convert To** and *Send to*.

??? question "How do I undo what an editor plugin did?"
    Every action first takes a checkpoint named “Before: action” and all its changes are one step. In the editor choose
    **Tools → Restore Checkpoint**. An action that fails leaves the book as it was.

??? question "How do I add a patch?"
    On the Extensions page press **Open patches folder** (in the *Patches* group), copy your `.toml` file into it and switch
    the patch on. Patches are off by default. An invalid patch is listed with the reason and cannot be switched on.

??? question "How do I try a patch without changing anything?"
    On its row open the **⋯** menu and choose **Dry Run on Your First Book**. It shows how many changes the patch would make
    and a before → after excerpt, using your first library book or a built-in sample. Nothing is changed.

??? question "In which order do patches apply, and when?"
    In the order shown in the Patches group; use **Apply Earlier** or **Apply Later** in the **⋯** menu to change it. Each patch
    has a moment: `import-after` changes the stored book, `convert-before` and `convert-after` only the copy you convert or
    send, `open-before` only what the reader shows.

??? question "Will my plugin keep working after an update?"
    Yes. A plugin declares the interface version it was written for and later Bookx releases keep running it unchanged.

??? question "Where do I learn to write a plugin or patch?"
    In the [developer guide](plugin-development.md). The folders can be moved with the `BOOKX_PLUGINS` and `BOOKX_PATCHES`
    environment variables.
