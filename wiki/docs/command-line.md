# Command line

`bookx-cli` does the library and conversion work without the GUI. Build it with
`cargo build --release -p bookx-cli` (see [Install](install.md#command-line-tool)). Run `bookx-cli --help` and
`bookx-cli <command> --help` for every option.

## Library location

The library is `--library DIR`, else `$BOOKX_LIBRARY`, else `~/Documents/Bookx Library` (the same default as the
app). Commands that change the library take its lock (`.bookx-lock`) and stop with a clear message when another
Bookx has it open.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | success |
| `1` | the command found problems (for example `check`) |
| `2` | usage or runtime error |

## Commands

| Command | What it does |
|---|---|
| `formats` | list the input and output formats (`--json`) |
| `presets` | list the built-in conversion presets |
| `convert INPUT OUTPUT` | convert a book; the format comes from the output extension |
| `meta show FILE` / `meta set FILE` | show or change metadata of a book file |
| `check FILE` | check a book for structural problems (`--json`); exit code 1 when errors are found |
| `device-check MOUNT` | validate the folder layout of a mounted e-reader and its books (`--kind`, `--book`, `--all`, `--json`); read-only, for a Kobo it also shows model, firmware and database facts |
| `library …` | manage the library, see below |
| `search QUERY` | search the library with the [query language](library.md#search) (`--sort`, `--desc`, `--json`) |
| `serve` | serve the library over OPDS and the web, with optional Bookx sync accounts |

### convert

```bash
bookx-cli formats
bookx-cli convert book.fb2 book.kepub.epub --smarten --device kobo
bookx-cli convert book.epub book.mobi --preset "Kindle (MOBI)"
bookx-cli convert book.epub book.txt --report      # what the format lost, what was changed
bookx-cli inspect book.azw3                        # metadata, contents tree, word/image/link counts
```

Presets: *Kindle (MOBI)*, *Kindle (AZW3)*, *Kobo (KEPUB)*, *Generic e-ink (EPUB 2)*, *Plain text to book*.
`--device` takes `kindle`, `kobo`, `eink` or `tablet`. Other options: `--epub-version 2|3`, `--smarten`,
`--unsmarten`, `--unwrap-lines [THRESHOLD]`, `--remove-unused-css`, `--remove-blank-paragraphs`,
`--remove-fake-margins`, `--auto-toc [force]`, `--split-large-html`, `--title`, `--author`, `--series`,
`--series-index`, `--tag`, `--language`, `--publisher`, `--cover FILE`, `--jobs N`, `-q`.

### meta

```bash
bookx-cli meta show book.epub --json
bookx-cli meta set book.epub --title "New" --author "A. Writer" --cover cover.png
bookx-cli meta set book.epub --identifier isbn:9781234567890 --rating 4
```

### check and search

```bash
bookx-cli check book.epub
bookx-cli search 'author:le guin rating:>=4' --json
```

### library

```bash
bookx-cli library add ~/Downloads -r        # other formats are converted to EPUB; --allow-duplicates
bookx-cli library list --json
bookx-cli library info ID
bookx-cli library remove ID --delete-files
bookx-cli library export ID ~/out.epub
bookx-cli library dump > catalogue.json     # every field of every book
bookx-cli library formats ID                # the format files of a book (main first)
bookx-cli library add-format ID book.azw3   # another format of a book; remove-format ID azw3
bookx-cli library merge-formats 1 2 3 --dry-run   # merge books 2 and 3 into 1 as formats; undo with `library undo`
bookx-cli library authors --json            # people instead of spellings; rename-author "A" "B" --to "C" --dry-run
```

Library health, the same features as the *Library Health* window:

```bash
bookx-cli library backup                    # quick snapshot of the catalogue on this computer
bookx-cli library snapshots
bookx-cli library restore latest
bookx-cli backup plan --to /mnt/nas/bookx   # how big the next backup to a place is (books, covers, catalogue)
bookx-cli backup run  --to /mnt/nas/bookx --encrypt   # only new or changed files are copied; see Backup and restore
bookx-cli backup journal --to /mnt/nas/bookx          # record changed highlights, notes and bookmarks
bookx-cli backup restore latest --to /mnt/nas/bookx --into ~/Books --dry-run   # also: list, verify [--deep], gc
bookx-cli library check --drift             # database, files, covers, file vs catalogue; --fix applies fixes
bookx-cli library import-calibre ~/Calibre\ Library --copy
bookx-cli library export-calibre ~/Exports/calibre   # a Calibre library Calibre can open; the folder must be empty
bookx-cli library layout flat --dry-run     # tree | flat | template --template '{author}/{title}' | linked
bookx-cli library trash list                # also: trash restore ID, trash empty
bookx-cli library history
bookx-cli library undo
bookx-cli library tidy --apply              # one spelling per author and series
bookx-cli library cite --style bibtex -o refs.bib "author:herbert"   # bibtex, ris, csl-json or apa
bookx-cli library fixity                    # SHA-256 check of the book files; exit 1 when one is damaged
bookx-cli library covers --json             # small or odd covers (--all: every cover with its size)
bookx-cli library inbox set ~/Downloads/books   # then: inbox scan | add-all | dismiss-all
```

`import-calibre` keeps the files in the Calibre folder unless you pass `--copy`. `export-calibre` writes
`Author/Title (id)/` folders with the book files, `cover.jpg` and `metadata.opf`, plus a `metadata.db`, into a
folder that does not exist yet or is empty (it refuses anything else, and a folder inside the library). It only reads
your library, prints its progress on standard error and exits with 1 when some files could not be copied.

### highlights

```bash
bookx-cli highlights repair --from /run/media/me/KOBOeReader   # what would change (nothing is changed)
bookx-cli highlights repair --from ~/backup/Kobo --apply       # correct colours and styles
bookx-cli highlights repair --from "Book.sdr/metadata.epub.lua" --json
```

Corrects the **colour and style** of highlights that are already in the library (for example imports that
all came in yellow) from what an e-reader says: a mounted e-reader or a copy of its folders (KOReader’s `.sdr`
sidecars and `.kobo/KoboReader.sqlite`), one KOReader `metadata.*.lua`, or a `KoboReader.sqlite`; the highlights the
Bookx sync server received and Bookx’s own sync records are always used when they exist (`--config-dir` names
another settings folder). Highlights are matched by their words; nothing is added or deleted, a colour you chose in
Bookx is left alone, and `--apply` writes one entry in the library’s history so `library undo` takes it back. Running it
again changes nothing. See [Colours and styles from your e-reader](annotations.md#colours-and-styles-from-your-e-reader).

### edit

```bash
bookx-cli edit book.epub OEBPS/ch1.xhtml --select-text "some words"
```

Starts Bookx with the book open in the editor (it is added to the library if needed). The names after the file are
opened in tabs; `--select-text` selects the first occurrence in the last one. `bookx --edit …` does the same.

### serve

```bash
bookx-cli serve --port 8080 --user me:secret                 # OPDS + web page
bookx-cli serve --user me:secret --kosync kosync.json        # plus KOReader progress sync
bookx-cli serve --tls-cert cert.pem --tls-key key.pem        # HTTPS
```

It binds to `127.0.0.1` by default; use `--bind 0.0.0.0` to reach it from other devices. Without `--user` access is
open, so always set a user when you bind beyond your own computer. `--read-only` and `--title` are also available.
To publish it (or `mcp --http`) under a name such as `https://bookx.lan/bookx`, see
[Put Bookx behind a reverse proxy](reverse-proxy.md).

### mcp

```bash
bookx-cli mcp                                    # Model Context Protocol over stdio (read-only)
bookx-cli mcp --http 127.0.0.1:8765 --token T    # Streamable HTTP with a bearer token
bookx-cli mcp --allow-write --allow-dir ~/Downloads
```

Lets AI assistants search and read the library. It is read-only unless `--allow-write` is given. See
[MCP (AI assistants)](ai-assistants.md) for the options, the tools and the safety rules.

!!! note "Limitations"
    - `meta set` edits EPUB files.
    - `bookx-cli` is not installed by `run.sh` or `make install`, and the Flatpak does not include it.
    - The output above reflects the current `--help`; options may be added over time.

## Questions and answers

??? question "How do I install bookx-cli?"
    Build it with `cargo build --release -p bookx-cli`. It is not installed by `run.sh` or `make install`, and the Flatpak
    does not include it. See [Install](install.md#command-line-tool).

??? question "Which library does it use?"
    `--library DIR`, else the `BOOKX_LIBRARY` variable, else `~/Documents/Bookx Library`, the same default as the app.
    Commands that change the library take its lock and stop with a message when Bookx has it open.

??? question "How do I convert a book?"
    `bookx-cli convert book.fb2 book.kepub.epub --smarten --device kobo`. The format comes from the output extension.
    `bookx-cli presets` lists ready-made presets, for example `--preset "Kindle (MOBI)"`, and `--report` says what the
    format lost.

??? question "How do I see or change a book's metadata?"
    `bookx-cli meta show book.epub --json` and `bookx-cli meta set book.epub --title "New" --author "A. Writer"`. Changing
    metadata edits EPUB files.

??? question "How do I check a book or a library?"
    `bookx-cli check book.epub` lists structural problems (exit code 1 when it finds errors).
    `bookx-cli library check --drift` does the same work as the Check Now button of main menu → Library Health → Health.

??? question "How do I make a backup or restore one from a terminal?"
    `bookx-cli backup run --to /mnt/nas/bookx --encrypt` copies the books, covers, catalogue and notes (only what is new
    after the first run) and `bookx-cli backup restore latest --to /mnt/nas/bookx --into ~/Books --mode replace-all`
    brings them back (add `--dry-run` to see what would happen). `backup plan`, `list`, `verify` and `journal` are
    there too.
    Passwords come from `BOOKX_WEBDAV_PASSWORD` and `BOOKX_BACKUP_PASSPHRASE`, never from the command line. See
    [Backup and restore](backup-and-restore.md#command-line).

??? question "How do I search from a script?"
    `bookx-cli search 'author:le guin rating:>=4' --json`. The query language is the same as in the library search.

??? question "How do I add a folder of books?"
    `bookx-cli library add ~/Downloads -r`. Other formats are converted to EPUB; `--allow-duplicates` keeps duplicates.

??? question "How do I check what is on my e-reader?"
    `bookx-cli device-check /run/media/me/KOBOeReader`, with `--all` for every book. It never writes to the device.

??? question "How do I run the library server without the app?"
    `bookx-cli serve --port 8080 --user me:secret --kosync kosync.json`. It listens on `127.0.0.1` unless you add `--bind
    0.0.0.0`, and you should always set `--user` when you do. The `--kosync` file keeps the Bookx sync accounts. To publish it
    under a name, use a [reverse proxy](reverse-proxy.md).

??? question "How do I let an AI assistant use my library?"
    `bookx-cli mcp` starts the read-only MCP server over stdio; the MCP page of Bookx Preferences (main menu → Preferences)
    has the setup for the app. Details are in [MCP](ai-assistants.md).

??? question "What do the exit codes mean?"
    `0` success, `1` the command found problems, `2` a usage or runtime error.
