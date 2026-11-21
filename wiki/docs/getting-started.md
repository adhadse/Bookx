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

To use another folder, open **Preferences** (++ctrl+comma++) and change the library location. Preferences has sections and a search (++ctrl+f++ inside it, see [Preferences](preferences.md)), and
the command palette (++ctrl+k++) finds every command, book and setting; see [Accessibility](accessibility.md).

## Adding books

<figure markdown="1">
![The Add menu](assets/library-add-menu-light.png#only-light)
![The Add menu](assets/library-add-menu-dark.png#only-dark)
<figcaption markdown="1">The **+** menu: files, the web, a folder, an audiobook, news subscriptions and a whole folder tree.</figcaption>
</figure>


- Click **+** in the header, then *Add Books* (++ctrl+o++), *Add Folder* (it also adds the books in the subfolders).
- Or drag files and folders onto the window.

Bookx reads the metadata and cover from every file. Formats other than EPUB are converted to EPUB on the way in,
see [Converting and exporting](converting.md). Duplicates are skipped. While a large import runs, a progress page
is shown.

Two more ways to get books in:

- **Add from the Web** turns a web page into a book, or a feed into a news book.
  See [News and Web](news-and-web.md).
- A **watched import folder** offers files that appear in a folder for review.
  See [Library](library.md#library-health).

## Importing a Calibre library

The **+** menu → *Import Calibre Library*. Pick the Calibre folder (the one that contains
`metadata.db`). You choose between **copying** the files into the Bookx library and **keeping them in the Calibre
folder**. A progress view runs, and at the end a report lists what was imported, what was skipped and what was
already there. The same is available on the command line:

```bash
bookx-cli library import-calibre ~/Calibre\ Library --copy
```

You can also go back: *Export as Calibre Library* in the Library menu writes a Calibre library that Calibre opens
([details](library.md#exporting-as-a-calibre-library)).

## Opening a book

Double-click a cover to read it. Right-click a book for the context menu: *Read*, *Edit Metadata*,
*Edit Book*, mark read or unread, *Convert To*, *Send by E-mail*, export annotations, save a copy, show in Files and
*Remove*.

Next: [Library](library.md), [Reading](reading.md).

!!! note "Limitations"
    - Calibre import reads `metadata.db` and imports books that have an EPUB file. Calibre books without an EPUB are
      counted as skipped in the report.
    - A book that has the same title and first author as one already in the library counts as already there.
    - It was tested with generated sample libraries, not with every Calibre version.
