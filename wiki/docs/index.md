---
title: Bookx, an e-book library, reader and editor for GNOME
description: Bookx is a free e-book library, reader and editor for GNOME, written in Rust. It keeps your books on your computer.
hide:
  - navigation
  - toc
  - footer
---

<div class="home" markdown>

<section class="hero" markdown>
<img class="hero-logo" src="assets/logo.svg" alt="" width="84" height="84">
<h1 class="hero-title">Your books, at home on GNOME</h1>
<p class="hero-lead">Bookx is a free e-book library, reader and editor. A native GTK 4 and libadwaita app, written in Rust. Your books stay on your computer.</p>


<div class="hero-actions" markdown>

[Install](install.md){ .md-button .md-button--primary }
[Documentation](getting-started.md){ .md-button }
[GitHub](https://github.com/adhadse/Bookx){ .md-button }

</div>
<p class="hero-note">Free software, GPL-3.0-or-later. Needs GNOME 49 or newer. Build from source; a Flatpak manifest is included.</p>
<div class="hero-shot">
<img class="only-light" src="assets/library-banner-hero-light.png#only-light" alt="The Bookx library: a Continue Reading banner above a grid of book covers, with the sidebar on the left" width="1612" height="1160" fetchpriority="high">
<img class="only-dark" src="assets/library-banner-hero-dark.png#only-dark" alt="The Bookx library: a Continue Reading banner above a grid of book covers, with the sidebar on the left" width="1612" height="1160" fetchpriority="high">
</div>

</section>

<section class="feature" markdown>
<div class="feature-text" markdown>
<p class="eyebrow">Library</p>

## A library that stays out of your way

Covers, progress and a **Continue Reading** card, and nothing else until you ask for it. Search with the Calibre
syntax, including `text:"words inside the book"`. Group books with shelves, tags and saved searches. Backups, a trash and
undo come with the library, and a Calibre library can be imported.

[More about the library](library.md){ .md-button }
</div>
<div class="feature-shot">
<img class="only-light" src="assets/library-details-light.png#only-light" alt="The library with a book selected and its details panel open" width="1392" height="880" loading="lazy" decoding="async">
<img class="only-dark" src="assets/library-details-dark.png#only-dark" alt="The library with a book selected and its details panel open" width="1392" height="880" loading="lazy" decoding="async">
</div>
</section>

<section class="feature reverse" markdown>
<div class="feature-text" markdown>
<p class="eyebrow">Reader</p>

## Pages made for reading

Bookx lays out the text itself: real pages in one or two columns, hyphenation, TeX-style line breaking, footnotes,
vertical Japanese and right-to-left text. Themes follow your system style, with sepia and black on top. The book map
shows where your highlights and bookmarks are.

[More about reading](reading.md){ .md-button }
</div>
<div class="feature-shot">
<img class="only-light" src="assets/reader-light.png#only-light" alt="The reader showing a page with highlighted passages and the selection popover open on a note" width="1392" height="880" loading="lazy" decoding="async">
<img class="only-dark" src="assets/reader-dark.png#only-dark" alt="The reader showing a page with highlighted passages and the selection popover open on a note" width="1392" height="880" loading="lazy" decoding="async">
</div>
</section>

<section class="feature" markdown>
<div class="feature-text" markdown>
<p class="eyebrow">Editor</p>

## Fix the book, not just read it

A built-in EPUB editor with a code view, a live preview drawn by the same engine as the reader, find and replace with
regular expressions, a contents editor and a book checker. Every tool takes a checkpoint first, so you can go back.

[More about editing](editing.md){ .md-button }
</div>
<div class="feature-shot">
<img class="only-light" src="assets/editor-preview-light.png#only-light" alt="The book editor with the file list, the code and the live preview" width="1392" height="880" loading="lazy" decoding="async">
<img class="only-dark" src="assets/editor-preview-dark.png#only-dark" alt="The book editor with the file list, the code and the live preview" width="1392" height="880" loading="lazy" decoding="async">
</div>
</section>

<section class="feature reverse" markdown>
<div class="feature-text" markdown>
<p class="eyebrow">Read aloud</p>

## Let it read to you

Bookx reads sentence by sentence and highlights the sentence being spoken. The natural voices run on your computer
after a one-time download; you can also point it at a voice server of your own.

[More about read aloud](read-aloud.md){ .md-button }
</div>
<div class="feature-shot">
<img class="only-light" src="assets/read-aloud-reading-light.png#only-light" alt="A page with the sentence being read aloud highlighted and the player card at the bottom" width="1392" height="880" loading="lazy" decoding="async">
<img class="only-dark" src="assets/read-aloud-reading-dark.png#only-dark" alt="A page with the sentence being read aloud highlighted and the player card at the bottom" width="1392" height="880" loading="lazy" decoding="async">
</div>
</section>

<section class="feature" markdown>
<div class="feature-text" markdown>
<p class="eyebrow">Annotations</p>

## Highlights and notes you can take with you

Five colours, four styles, notes and bookmarks. See everything you marked across the library in one view, export it
as Markdown, text or JSON, or send it to your e-reader together with the book.

[More about annotations](annotations.md){ .md-button }
</div>
<div class="feature-shot">
<img class="only-light" src="assets/library-notes-light.png#only-light" alt="The Notes and Highlights view listing highlights from several books" width="1392" height="880" loading="lazy" decoding="async">
<img class="only-dark" src="assets/library-notes-dark.png#only-dark" alt="The Notes and Highlights view listing highlights from several books" width="1392" height="880" loading="lazy" decoding="async">
</div>
</section>

<section class="feature reverse" markdown>
<div class="feature-text" markdown>
<p class="eyebrow">Devices and sync</p>

## Your e-reader and your other apps

Send books to an e-reader over USB, to a phone, a WebDAV folder or by e-mail, with the right format for the device.
Serve the library over OPDS, and keep your reading position in step with KOReader. Device support is new: so far it has
been tried on real hardware with one Kobo only.

[More about devices](devices-and-sharing.md){ .md-button }
</div>
<div class="feature-shot">
<img class="only-light" src="assets/device-window-light.png#only-light" alt="The Device window listing the books on a connected e-reader" width="1392" height="880" loading="lazy" decoding="async">
<img class="only-dark" src="assets/device-window-dark.png#only-dark" alt="The Device window listing the books on a connected e-reader" width="1392" height="880" loading="lazy" decoding="async">
</div>
</section>

<h2 class="section-title">Everything else</h2>
<p class="section-lead">The details that make a library worth keeping.</p>

<div class="grid cards" markdown>

- :material-bookshelf: **Calibre import**

    ---

    Bring your Calibre library over, keeping the files where they are or copying them in. [Getting started](getting-started.md)

- :material-magnify: **Search inside books**

    ---

    A full-text index of your books, built in the background, next to the Calibre search syntax. [Search](library.md#search)

- :material-backup-restore: **Backups and undo**

    ---

    Catalogue snapshots, restore, a trash and an undo list. [Backup and restore](backup-and-restore.md)

- :material-swap-horizontal: **Convert**

    ---

    Read many formats, write KEPUB, AZW3, MOBI, FB2 and more, from the app or the command line. [Converting](converting.md)

- :material-sync: **Bookx sync**

    ---

    Bookx sync keeps position, highlights, notes and bookmarks in step with KOReader, on your own network. [Bookx sync](progress-sync.md)

- :material-newspaper-variant-outline: **News and web pages**

    ---

    Turn feeds and web pages into books on a schedule. [News and Web](news-and-web.md)

- :material-puzzle-outline: **Plugins and patches**

    ---

    Sandboxed WebAssembly plugins and small regex patches, off until you switch them on. [Plugins](plugins-and-patches.md)

- :material-console: **Command line**

    ---

    `bookx-cli` converts, checks, searches and serves without the app. [Command line](command-line.md)

- :material-keyboard-outline: **Keyboard first**

    ---

    A command palette, customisable shortcuts and a screen-reader-aware reader. [Shortcuts](shortcuts.md), [Accessibility](accessibility.md)

</div>

<h2 class="section-title">Formats</h2>
<p class="section-lead">Everything is converted to EPUB when you add it, and your files stay yours.</p>

<div class="formats">
<span>EPUB</span><span>KEPUB</span><span>MOBI</span><span>AZW3</span><span>FB2</span><span>PDF</span><span>DOCX</span><span>ODT</span><span>RTF</span><span>CBZ</span><span>CBR</span><span>HTMLZ</span><span>TXT</span><span>Markdown</span><span>PDB</span><span>LRF</span><span>CHM</span>
</div>
<p class="formats-note">Files with copy protection are not converted. PDF is read page by page, with limits. See <a href="converting/">the full table</a>.</p>

<h2 class="section-title">Private and open</h2>
<p class="section-lead">No account, no cloud, no tracking. Bookx talks to the internet only when you ask it to: fetching metadata, a dictionary lookup, a news feed, an e-mail. It is free software under the GPL-3.0-or-later, and you can read, change and share the code.</p>

<div class="cta" markdown>
<h2 class="section-title" style="margin-top:0">Try Bookx</h2>
<p class="section-lead">Build it from source, or use the Flatpak manifest. Prebuilt packages are not published yet.</p>

[Install](install.md){ .md-button .md-button--primary }
[Read the guide](getting-started.md){ .md-button }

</div>

<h2 class="section-title">Community</h2>
<p class="section-lead">Questions, bugs and ideas are welcome on <a href="https://github.com/adhadse/Bookx/issues">GitHub</a>. You can help with <a href="translating/">translations</a>, <a href="plugin-development/">plugins</a> or code: see <a href="contributing/">Contributing</a>.</p>

</div>
