# Highlights and notes: import and export

Your highlights, notes and bookmarks are not trapped in Bookx. You can bring in the ones you made on a Kindle, in
KOReader, on a Kobo or in calibre, and take yours out as Markdown, HTML, JSON, an Obsidian note or Readwise data.

## Notes & Highlights

*Sidebar → **Notes & Highlights*** shows every highlight and note of every book in the main area of the library.

- **Search** looks in the quote, your note, the book title, the author and the chapter. Every word you type has to
  be found (any case).
- **Group** by book, month, colour or book tag, or not at all. **Order**: newest first, oldest first, or in book order.
- **Filters**: colour, a book, with / without a note, and a date range (`YYYY-MM-DD`, both days included).
- **Click a row** to open the book at that passage. For a PDF the row shows the page (`page 12`), for other books
  the chapter.
- The pencil button edits the note; the bin deletes the highlight (after asking). Both change the book.
- Tick the check boxes to choose highlights, or leave nothing ticked to use everything shown. The **Export**
  button then offers every format below, Anki flashcards and Readwise.
- The list shows the first 200 matches; *Show more* adds the next ones.

## Highlights are kept where their text is

Every highlight is stored with its **text** and a position in the chapter; the text is the truth. When you open a
library for the first time with this version, Bookx re-checks every stored highlight against its book once: one
whose position no longer holds its text (older imports from a device were numbered another way and were painted a
few characters to a few hundred characters late, starting and ending in the wrong words) is moved to where its
text really is, and one whose text is nowhere in the book becomes a note without a place (see below). Nothing is
lost: the text, colour and note stay, the old position stays in the record, and the whole pass is **one step in the
library’s history: Undo restores every highlight as it was.**

## Colours and styles from your e-reader

A highlight keeps the colour and style it had on the device, and a re-import or sync keeps them up to date:

| | Kobo | KOReader | Bookx |
|---|---|---|---|
| yellow | colour 0 | yellow | yellow |
| pink | colour 1 | (red) | pink |
| blue | colour 2 | blue | blue |
| green | colour 3 | green | green |
| red, orange, purple, gray | (nearest of the four) | the same names | the same names |
| olive, cyan | — | olive, cyan | **kept as written**, drawn in a nearby shade (green, blue) |
| highlight (background) | — | lighten | background |
| underline | — | underscore | underline |
| strikethrough | — | strikeout | strikethrough |
| invert | — | invert | invert |

Kindle’s *My Clippings.txt* has no colours: those highlights come in yellow. A colour Bookx has no swatch for is stored as the device wrote it, so a later export or a note file written for KOReader gives it back unchanged.

- **Two highlights of the same words in different colours stay two highlights.**
- **What you change in Bookx stays.** Bookx remembers what the device said about each highlight. A re-import that says the same changes nothing; if the device changed the colour since, the device’s colour is taken.
- **Older imports are corrected.** Highlights imported by an earlier version were all yellow; the next time Bookx reads the device (sync or *Import Highlights*) it gives them the device’s colour and style, as **one undoable step** in the library’s history. A highlight whose colour you chose yourself is not touched.
- *Notes & Highlights* shows a **colour legend** (each colour with its count) and the **Colour** filter and *Group by colour* narrow the list; exports keep colour and style (JSON, HTML, Markdown/Obsidian callouts).

## Import Highlights

<figure markdown="1">
![The Import Highlights dialog](assets/library-import-highlights-light.png#only-light)
![The Import Highlights dialog](assets/library-import-highlights-dark.png#only-dark)
<figcaption markdown="1">Import Highlights: files, or a folder or device. You see what would be added before anything happens.</figcaption>
</figure>


*Main menu → **Import Highlights*** (also in the right-click menu of a book). Choose **Files** or
**Folder or Device**; Bookx reads what it finds and shows a **review before anything is added**:

> **Add 3 highlights to your library?**
> UI Test Book: 3 highlights added, 1 not found in the book’s text, added as notes without a place
> Left out, not in your library: Some Other Book (1)
> Nothing is deleted, nothing you already have is changed, no quote is added twice.

What it reads:

| Source | What to choose | Notes |
|---|---|---|
| **Kindle** | `My Clippings.txt` (on the Kindle in `documents/`) | Highlights and notes; a note is joined to the highlight at its location. Bookmarks are skipped. Dates are read when the Kindle wrote them in English. |
| **KOReader** | a book’s `metadata.*.lua` (in its `.sdr` folder), the folder of a device or KOReader, or KOReader’s own export (*Export highlights* as JSON or Markdown) | Title and author come from the book’s properties, else from the `.sdr` folder name. |
| **Kobo** | a copy of `KoboReader.sqlite`, or the Kobo’s folder (the mounted device) | The database is only **read, from a temporary copy**; the file on the Kobo is not opened. Highlights and notes of every book. |
| **calibre** | an annotations export (JSON) from calibre’s viewer or `calibredb annotations export` | Read leniently; if a file is not recognised you get a message. |
| **Bookx** | the JSON of *Export Annotations* | Handy to move highlights between libraries. |

How it decides:

- **Books are matched by title** (case, punctuation and a number prefix like `01 -` ignored), with the author as
  tie-break and veto. A book that is not in your library, or that fits two of yours, is **listed and left out**, never guessed.
- A quote you **already have** (same words, ignoring case and punctuation, or one inside the other) is **not added
  again**. If your copy has no note and the imported one has, the note is filled in; **a note you wrote is never
  overwritten**.
- **The text decides where a highlight goes.** Bookx never trusts the positions a device writes (a Kobo’s paths and
  offsets, a KOReader xpointer, a Kindle location): they describe the device’s own rendering of the book. It searches
  the book for the quote itself: exactly, then ignoring curly or straight quotes, dashes, soft hyphens, ligatures,
  line breaks and extra spaces, then ignoring case, then allowing small differences (a missing comma, a changed word
  at an end). If a quote is in several chapters, the chapter the device names (when it names one) and the one
  closest to it win. The highlight is stored where that text really is, so what you see on the page is exactly the
  text the device had.
- **A quote that cannot be found is never painted at a guess.** It is added as a **note without a place**: it keeps
  its text and your note, shows in *Notes & Highlights* and in the reader’s notes list marked *Could not be located in the book*,
  and is not drawn on any page. Highlights from a different edition of the book are often like this. In *Notes &
  Highlights*, its menu has **Locate again**, which searches the book once more (after you changed the file or
  fixed the title); if the text is not in this book, select the passage and highlight it yourself and delete the note.
- Bookmarks and notes without a highlighted quote have no place to attach to and are not imported.
- It only ever **adds**. Importing the same file twice adds nothing the second time.

## Export Annotations

<figure markdown="1">
![The Export All Annotations dialog](assets/library-export-annotations-light.png#only-light)
![The Export All Annotations dialog](assets/library-export-annotations-dark.png#only-dark)
<figcaption markdown="1">Export All Annotations: pick Markdown or another format, then the folder.</figcaption>
</figure>


- **One book:** right-click → *Export Annotations* → a format.
- **The whole library:** *Main menu → **Export All Annotations*** → a format → a folder. One file per book that has
  highlights, notes or bookmarks, named `<title> - annotations.<ext>`.

| Format | Use it for |
|---|---|
| Markdown, Plain text, HTML | reading, printing, notes apps |
| JSON | other tools; Bookx can import it again |
| Obsidian note | a note with a quote callout per highlight, ready for a vault |
| Kindle clippings | a `My Clippings.txt`-style file for tools that read Kindle exports |
| Readwise | the JSON body of Readwise’s highlights API, if you want to send it yourself; see *Send to Readwise* below to let Bookx do it |

Every highlight in the JSON export carries an `id` that comes from the book and the quote (`bx-…`), so it is the
same on every export and on every computer. Importing a file you exported never adds a quote twice.

Highlights come in book order with their chapter and note; bookmarks follow. A book’s own Bookx JSON is different
from calibre’s or KOReader’s; Bookx reads all of them.

### Anki flashcards

In Notes & Highlights, **Export → Anki flashcards** makes cards from the highlights shown or ticked, or from
your [vocabulary](reading.md) list.

| Choice | What you get |
|---|---|
| Anki package (`.apkg`) | open it in Anki with *File → Import*; a deck *Bookx::Highlights* or *Bookx::Vocabulary* is made |
| Anki text file / CSV | for *Import File*; the header lines tell Anki the deck, the note type and where the id and tags are |

*Front/back* cards show the quote on the front and your note and the source (book, author, chapter) on the back.
*Cloze* cards hide the longest word of the quote. Vocabulary cards show the word and its meaning, with the sentence you
met it in (the word in bold, or hidden for a cloze). Each card is tagged `bookx` and with the book. Cards have a fixed
id, so importing a newer export updates the cards in Anki instead of adding copies.

The `.apkg` was checked by reading it back, not by importing it into every Anki version; if Anki complains, use the text
file.

### Send to Readwise

**Export → Send to Readwise** uploads the highlights (those ticked, or everything shown) to your Readwise library.

- The first time, paste your access token (from readwise.io/access_token). It is kept in the system keyring, not in
  the settings file. *Readwise Access Token* in the same menu changes it.
- Bookx remembers what it sent. A second send uploads only new highlights and the ones whose note changed.
- There is a progress bar and a Cancel button; what was sent before you cancelled is remembered.
- Errors are shown in words: a token that is not accepted, or Readwise asking you to slow down (try again in a minute).

Bookmarks and highlights without text are not sent.

### Kindle

With a Kindle connected, *Import Highlights → Folder or Device* reads `documents/My Clippings.txt` from it. Nothing
on the Kindle is changed. The Kindle’s own per-book annotation files (`.sdr` folders with `.azw3r` or `.mbp` files)
are a binary format that Bookx does not read; the clippings file has the same highlights and notes.

See also [Devices and sharing](devices-and-sharing.md) for syncing notes with a connected e-reader and
[Sync your reading position](progress-sync.md) for the position.
