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

## Import Highlights

*Main menu → **Import Highlights*** (also in the right-click menu of a book). Choose **Files** or
**Folder or Device**; Bookx reads what it finds and shows a **review before anything is added**:

> **Add 3 highlights to your library?**
> UI Test Book: 3 highlights added, 1 not found in the book’s text, left out
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
- The quote is placed in the book by searching for its text (curly or straight quotes, dashes and line breaks do not
  matter). A quote that cannot be found in the book’s text is **left out** and counted in the review: highlights from
  a different edition of the book often can’t be placed.
- Bookmarks and notes without a highlighted quote have no place to attach to and are not imported.
- It only ever **adds**. Importing the same file twice adds nothing the second time.

## Export Annotations

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
