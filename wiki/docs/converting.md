# Converting and exporting

EPUB is Bookx's native format. Other formats are **converted to EPUB when you add them**, and EPUB books can be
converted to other formats on the way out.

## Input formats

| Format | Extensions | Status |
|---|---|---|
| EPUB | `epub` | native |
| FictionBook | `fb2`, `fbz` | supported |
| MOBI / Kindle | `mobi`, `azw`, `azw3`, `prc` | supported; files with DRM are recognised and refused with a clear message |
| Word, OpenDocument, RTF | `docx`, `odt`, `ott`, `rtf` | supported |
| HTML archives, text archives | `htmlz`, `txtz` | supported |
| Plain text, Markdown (with front matter), HTML | `txt`, `md`, `markdown`, `html`, `htm`, `xhtml` | supported |
| Comics | `cbz`, `cbr`, `cbt`, `cbc` | supported |
| eReader, Palm, others | `pdb`, `pml`, `pmlz`, `rb`, `snb`, `tcr`, `chm` | supported |
| Sony LRF | `lrf` | best effort: text, images and contents; no page layout, fonts, buttons or sound; encrypted LRX is rejected |
| Kindle Print Replica | `azw4` | only when it is an ordinary MOBI inside; replicas (a PDF inside) are rejected |
| PDF | `pdf` | **supported with limits**: read page by page in the reader; the text is also reflowed into an EPUB (experimental). Scans without text, password-protected files and fixed layouts: see [PDF](reading.md#pdf) |
| Microsoft Reader | `lit` | **not supported**: discontinued format, recognised and rejected |

## Output formats (Convert To)

Right-click a book → *Convert To* and pick a format.

| Format | Extension | For |
|---|---|---|
| EPUB (cleaned up) | `epub` | a tidied copy |
| KEPUB | `kepub.epub` | Kobo |
| AZW3, MOBI | `azw3`, `mobi` | Kindle |
| FictionBook | `fb2` | |
| HTML archive, text archive | `htmlz`, `txtz` | |
| Word, RTF | `docx`, `rtf` | |
| eReader, PML archive, Rocket eBook, Bambook SNB | `pdb`, `pmlz`, `rb`, `snb` | older readers |
| OEB folder as a zip | `oeb`, `zip` | |
| Plain text, Markdown, HTML | `txt`, `md`, `html` | |

Plug-ins can add more entries, shown as "name (.ext, plugin)". See [Plugins and patches](plugins-and-patches.md).
Right-click also offers *Save a Copy* and *Show in Files*.

When you send a book to a device, Bookx converts it to the device's format automatically. A book that holds
several [formats](library.md#formats-of-one-book) sends the one that suits the device (KEPUB to a Kobo, AZW3 or MOBI to a
Kindle, EPUB to the rest) without converting; conversion starts from the EPUB.

Bookx does not convert files with copy protection (see [Copy protection](library.md#copy-protection)). If a conversion fails it
tries the next format the device opens. See [Devices and sharing](devices-and-sharing.md).

### How well each format converts

Bookx's converters are compared with Calibre's on a set of test books (EPUB 2/3, FictionBook, Word, ODT, RTF,
Markdown, HTML, text in several encodings, comics and files made by Calibre) for every pair of formats. A format
"keeps" a feature when a book converted into it and read back still has it.

| Output | Pictures | Cover | Contents | Nested contents | Links and footnotes | Series |
|---|---|---|---|---|---|---|
| EPUB, KEPUB, AZW3 | yes | yes | yes | yes | yes | yes |
| MOBI | yes | yes | yes | yes | yes | yes |
| FictionBook | yes | yes | yes | by sections | yes | yes |
| Word | yes | on the first page | yes | yes | yes | yes |
| HTML archive | yes | yes | no (one page) | no | yes | yes |
| Text archive | no | yes | no | no | no | yes |
| Plain text, Markdown | no | no | no | no | no (Markdown: links) | no |
| HTML (single file) | no | no | no | no | yes | no |
| RTF | no | no | no | no | no | no |
| eReader (PDB) | yes | no | yes | no | no | no |
| PML archive, Rocket eBook, Bambook | yes | yes | yes | no | yes | no |

Parity with Calibre: for the test books no format pair loses more than Calibre does. Known small differences: text
output starts with the title and author; Bookx does not invent a cover, a title page or a contents page for books
that have none; Calibre's own PML, RTF and text files glue list items and table cells into one word, which no reader
can undo. Details for contributors are in `docs/convert-parity.md`.

## Check before sending or converting

Right-click one or more books → *Check Before Sending*. Bookx runs the EPUB validator and the rules of the place the
book is going to (choose *Any device*, *Kindle*, *Kobo* or *E-mail* at the top) and lists, per book, the **blocking
problems** (the device or converter will refuse or break the book: a broken `mimetype`, no language for Amazon,
DRM, a file over 25 MB for e-mail) and **warnings** (no cover, no author, validator warnings). Problems that can be
repaired automatically are marked *can be fixed*; **Fix All** repairs them and keeps the original of each book in the
library's `.bookx/preflight-backups` folder.

Switch on *Check automatically before converting or e-mailing* in the same window to make this a step of *Convert To*
and *Send by E-mail*: the window only appears when something was found, and *Continue Anyway* goes on. Sending
to a device in the Device window does not run this check yet.

## Conversion report

After *Convert To*, a small text file `<output>.report.txt` is saved next to the converted file. It names the target,
the size before and after, what the checker found in the source book and (for EPUB and KEPUB results) in the result,
and says when embedded fonts or SVG images may not be kept in the chosen format. Switch it off with *Save a report
next to converted files* in the *Check Before Sending* window. On the command line, `bookx-cli convert in.epub out.txt
--report` also lists what the output format cannot hold (pictures, cover, contents, links, series) and what the
conversion steps changed; `--report FILE` saves it (a `.json` name gives JSON).

## Command line

`bookx-cli convert` picks the output format from the output extension:

```bash
bookx-cli formats
bookx-cli convert book.fb2 book.kepub.epub --smarten --device kobo
bookx-cli convert book.epub book.mobi --preset "Kindle (MOBI)"
bookx-cli presets
```

Options include `--smarten`/`--unsmarten`, `--unwrap-lines`, `--remove-unused-css`, `--remove-blank-paragraphs`,
`--remove-fake-margins`, `--auto-toc`, `--split-large-html`, `--epub-version 2|3`, and metadata overrides such as
`--title`, `--author`, `--series` and `--cover`. See [Command line](command-line.md).

## Exporting annotations

Right-click → *Export Annotations* writes your highlights, notes and bookmarks as Markdown, HTML, JSON, plain text,
an Obsidian note, Kindle clippings or Readwise data; *Main menu → Export All Annotations* does it for the whole
library. *Import Highlights* brings in the ones from a Kindle, KOReader, a Kobo or calibre. See
[Highlights and notes](annotations.md).

!!! note "Limitations"
    - Converting **from** PDF reflows the text into an EPUB (experimental: columns, tables, formulas and images are not
      reconstructed, scanned PDFs have no text to convert). Converting **to** PDF is not available.
    - Conversion works on reflowable content; layout-heavy books (fixed layout, comics with text) can lose
      layout in text-based outputs.
    - Output for Kindle and Kobo formats is checked by structural tests and `bookx-cli device-check`, not yet on
      real devices. See [Devices and sharing](devices-and-sharing.md).
    - DRM-protected books are not supported.
