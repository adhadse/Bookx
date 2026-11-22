# Getting started

## Your first library

<figure markdown="1">
![The welcome page of an empty library](assets/library-welcome-light.png#only-light)
![The welcome page of an empty library](assets/library-welcome-dark.png#only-dark)
<figcaption markdown="1">A new library: the welcome page asks you to add books, and nothing else gets in the way.</figcaption>
</figure>


When you start Bookx for the first time the library is empty and shows a welcome page. The library is a plain
folder, by default `~/Documents/Bookx Library`. It holds your book files and a SQLite catalogue,
`bookx-library.db`. You can move the folder, copy it or back it up like any other folder.

The paths here are for a native install. The Flatpak bundle keeps its settings under `~/.var/app/com.adhadse.Bookx/` and uses the same library folder; see [Install](install.md#where-things-are-stored).

To use another folder, open the main menu (the hamburger button at the top right of the library, ++f10++), choose **Preferences** (++ctrl+comma++), go to *Library* and press *Change* next to *Library folder*. Preferences has sections and a search (++ctrl+f++ inside it, see [Preferences](preferences.md)), and
the command palette (++ctrl+k++) finds every command, book and setting; see [Accessibility](accessibility.md).

## Adding books

<figure markdown="1">
![The Add menu](assets/library-add-menu-light.png#only-light)
![The Add menu](assets/library-add-menu-dark.png#only-dark)
<figcaption markdown="1">The **+** menu at the top of the sidebar: books, the web, a folder, an audiobook, and the two imports (a Calibre library and highlights).</figcaption>
</figure>


- Click the **+** button at the top of the sidebar, then *Add Books* (++ctrl+o++) or *Add Folder* (it also adds the books in the subfolders and tells you how many it found).
- Or drag files and folders onto the window.

Bookx reads the metadata and cover from every file. Formats other than EPUB are converted to EPUB on the way in,
see [Converting and exporting](converting.md). Duplicates are skipped. While a large import runs, a progress page
is shown.

Two more ways to get books in:

- **+** → *Add from the Web* turns a web page into a book, or a feed into a news book.
  See [News and Web](news-and-web.md).
- A **watched import folder** offers files that appear in a folder for review.
  Set it up in *Library Health* (the entry at the bottom of the sidebar) → *Health*; see [Library](library.md#library-health).

## Importing a Calibre library

Click the **+** button at the top of the sidebar and choose *Import Calibre Library*. Pick the Calibre folder (the one that contains
`metadata.db`). You choose between **copying** the files into the Bookx library and **keeping them in the Calibre
folder**. A progress view runs, and at the end a report lists what was imported, what was skipped and what was
already there. The same is available on the command line:

```bash
bookx-cli library import-calibre ~/Calibre\ Library --copy
```

You can also go back: *Export as Calibre Library* in the **Library** menu (the button next to *View options* in the header bar) writes a Calibre library that Calibre opens
([details](library.md#exporting-as-a-calibre-library)).

## Opening a book

Double-click a cover to read it. Right-click a book for the context menu: *Read*, *Edit Metadata*,
*Edit Book*, *Reading Status*, *Send To* (device, e-mail, save a copy), *Convert To*, *Export Annotations*, show in Files and
*Move to Trash* (a message with **Undo** follows).

Next: [Library](library.md), [Reading](reading.md).

!!! note "Limitations"
    - Calibre import reads `metadata.db` and imports books that have an EPUB file. Calibre books without an EPUB are
      counted as skipped in the report.
    - A book that has the same title and first author as one already in the library counts as already there.
    - It was tested with generated sample libraries, not with every Calibre version.

## Questions and answers

??? question "How do I add my first books?"
    Click the **+** button at the top of the sidebar and choose *Add Books* or *Add Folder*, or drag files and folders
    onto the window. The welcome page of an empty library has the same buttons.

??? question "Where is my library, and can I move it?"
    By default in `~/Documents/Bookx Library`. It is a normal folder. To use another one, open the main menu →
    *Preferences* → *Library* → *Change* next to *Library folder*; or use the library switcher at the top left of the
    sidebar to keep several libraries.

??? question "I used Calibre. Can I keep my books?"
    Yes: **+** → *Import Calibre Library* and pick the Calibre folder. You can copy the files or leave them where they
    are. Books without an EPUB are skipped and listed in the report.

??? question "How do I open a book?"
    Double-click its cover, or right-click it and choose *Read*. Bookx opens it where you stopped.

??? question "Where are all the settings and commands?"
    Main menu (the hamburger button) → *Preferences* for settings, *Keyboard Shortcuts* for keys and *Help* for this
    guide. The **Library** menu next to *View options* holds statistics, vocabulary and the catalogues. ++ctrl+k++ opens
    the command palette, which finds any of them by name.

??? question "Where do I find the Bookx command line?"
    `bookx-cli` is a separate program, see [Command line](command-line.md).
