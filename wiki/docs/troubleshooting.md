# Troubleshooting and FAQ

!!! warning "Beta"

    Bookx 0.5.0-beta.1 is beta software. It can have bugs, so keep a backup of anything you care about (Bookx can back up your library for you). Reports help us fix bugs: choose **Main Menu → Report a Problem**, read the log report, then **Open GitHub Issue**.

## How to report a problem

1. Open **Main Menu → Report a Problem**. From a window of a feature the same row is there too: the **Sync** window, the **Devices** window, the reader menu and the editor menu. The report then starts with that feature's section.
2. Read the report it shows. Everything that would be shared is in the preview, and you can edit the "What happened?" text.
3. Choose **Open GitHub Issue** (the report is put on your clipboard first; paste it into the form), **Copy Report** or **Save Report**.

Bookx sends nothing itself. A report is made on your computer and goes where you put it.

### What is in the report { #what-is-in-the-report }

A report starts with a **Summary and suspected area**: what failed last, which part of Bookx it belongs to, and a hint, so whoever reads it sees the likely cause at once. Then, in this order:

- **Recent operations**: the last things the app did in each area (sync, import, conversion, devices, backups, reading aloud, search, plugins, the built-in server) with what it was, how long it took, whether it worked and which kind of error it was (for example "the server answered 404", "no space left", "not mounted"). Never a title, a file name or a folder.
- **Recent problems**: the warnings and errors of the log, grouped and counted. Harmless repeated desktop notices are counted and left out, so they do not push the useful lines out.
- **The state of each feature**: your sync server address as its kind, port and path words only (`http://<ip:private>:30260/api/koreader`), whether you are logged in, how documents are matched, the devices that connected (by model, such as "Boox Leaf 2", never by the name you gave them), backup places by kind, read-aloud engine and helper programs, plugins by id and version.
- **Your computer**: version and installation (Flatpak permissions such as network access, in words), system and desktop, display and renderer, processors, memory as a class, time zone offset, free space and file system of the library folder (for example "network file system", "synced by Syncthing"), how large the catalogue is and whether it passes its check.
- **The books involved**: for the books an action was about, their format, size class and reading progress. Titles and authors only if you switch **Include book titles and authors of the books involved** on (off by default).
- The **recent log**, with the important lines kept first when it is too long, and the **last crash** if Bookx closed unexpectedly.

**Never included:** your name, user name, e-mail address, IP addresses, Wi-Fi name, passwords and tokens, folders and file names, the text of your books and notes, search words. The host of a server address is left out unless you switch **Include the server address host** on (off by default); the user name, password and query of an address are never shown. A document id (the hash of a book file that sync uses) is shown: it identifies the bytes of a file, not you.

The text you type under **What happened?** is shared exactly as you type it, so leave out anything private. The whole report is limited to 60 KB.

## The library opens read-only

Another running Bookx (or `bookx-cli`) has the library open. The message names who has it. Close the other one and
reopen the library. A lock left behind by a crashed Bookx on the same computer is taken over automatically; a lock
from another computer never is, so make sure nothing else is using the library before you remove `.bookx-lock`.

## I get a warning about a network share or a sync folder

A library on a network file system, or in a Dropbox/Syncthing-style folder, can be damaged if two computers change it
at once. Keep the library on a local disk and use [backups](library.md#library-health) or the
[OPDS server](devices-and-sharing.md#library-server) to reach it from elsewhere.

## A book did not import

- LIT is not supported, and Bookx tells you so. A PDF imports: its pages open in the PDF view and its text is reflowed into an EPUB. A scanned PDF has no text to search or reflow (there is no OCR yet); a password-protected PDF imports and asks for the password when you open it.
- DRM-protected books cannot be read.
- Duplicates are skipped silently. Open the main menu → **Library Health** and pick **Duplicates** to see copies.
- Run `bookx-cli check book.epub` to see what is wrong with an EPUB, or open it in the [editor](editing.md) and press
  ++f7++.

## Something looks wrong after a metadata change or layout change

Open the main menu → **Library Health**. Its **Trash** page has a **History** list with **Undo**, and a deleted book waits
in the trash below it. A backup can be restored from the **Backups** page. See [Library](library.md#library-health).

## The library check reports missing files

Open the main menu → **Library Health**, run the check on its **Health** page and read what each problem offers. Removing a missing file only removes it from
the catalogue (into the trash); it does not touch other files.

## Read aloud does nothing

Open the speaker popover: the line under the buttons says which voice is reading and why it is not the one you chose.

* *Kokoro is not downloaded* / *no Piper voice is downloaded*: download it in Bookx Preferences (main menu → Preferences) on the **Text to Speech** page.
* *espeak-ng is not installed*: Kokoro and Piper need the `espeak-ng` package.
* *no audio player found*: install PulseAudio or PipeWire tools (`pacat`, `pw-cat`) or `alsa-utils` (`aplay`).
* *cannot reach the voice server*: start the server, check the address, use **Test** on the settings page.
* *Preparing voice* stays at 0% or says you are offline: the default voice (about 130 MB) is downloaded the first time you
  press Read Aloud. Connect to the internet and press *Try again* (the part already downloaded is kept), or choose
  an external voice server on the same **Text to Speech** page.
* A natural voice that is slower than speech (an old computer, `BOOKX_TTS_THREADS=1`) stutters at the start of each
  sentence: use Piper, a smaller model, or a server on a faster machine.

More in [Read aloud](read-aloud.md).

## There are no dictionaries or the spelling check finds nothing

The reader looks for StarDict dictionaries in `~/.local/share/bookx/dictionaries/`. The editor's spelling check uses
the Hunspell dictionaries installed on the system. Install a dictionary package for your language.

## Wikipedia or translation does not respond

These look-ups go to the web. Check your network connection.

## My device does not show up

- Make sure it is mounted (GNOME Files shows it). For phones, unlock the phone and choose file transfer.
- A USB stick, a card reader or an e-reader Bookx does not recognise is never offered as a device by itself. Open the Devices window, **Settings** page, **Other drives**: it lists such drives, and **Use as an e-reader** makes Bookx treat one as a reader. See [How Bookx recognises a device](devices-and-sharing.md#recognition).
- Run `bookx-cli device-check <mount>` and look at what it reports.
- Device support has been tried on real hardware with one Kobo (a Kobo Libra Colour) only; everything else is tested with simulated devices. Please report what you see, including the output of
  `bookx-cli device-check --json <mount>`.

## Where are my settings and data?

See [Install](install.md#where-things-are-stored). Reading positions, highlights and bookmarks live in the library
database, so they travel with the library folder.

## Does Bookx send anything to the internet?

Only when you ask it to: fetching metadata, Wikipedia and translation, *Add from the Web*, news downloads, sending
e-mail, and plugins that declare network permissions. Plugins are off until you enable them.

## Copy diagnostics

*Main menu → About Bookx → Troubleshooting* shows the same text as the command palette entry **Copy Diagnostics**
(++ctrl+shift+p++, type "diagnostics"), which puts it on the clipboard. It lists:

- the Bookx version and whether it runs as a Flatpak, your distribution, desktop and session type;
- the GTK and libadwaita versions, the interface language and locale;
- where the library and the settings file are (your home folder is shown as `~`), how many books the library holds;
- which features are on (E-ink mode, library sharing, automatic backups, footnote popups, the read-aloud engine) and
  how many plugins and patches are installed.

It contains **no** book titles, authors, notes, account names, passwords, tokens, addresses or your user name: those are
removed before the text is shown. Read it once before you paste it into an issue.

## Debug log

The **Troubleshooting** page of *About Bookx* also lists the last log lines of this run. Before you save or copy it,
Bookx removes your home folder and user name, the library folder, the titles of your books, e-mail and IP addresses,
and every password or token you have saved. Nothing is written to disk until you press Save.

## Safe mode

If Bookx crashes at start, run `bookx --safe-mode` (Flatpak: `flatpak run com.adhadse.Bookx --safe-mode`; or set
`BOOKX_SAFE_MODE=1`). Plugins and patches are not loaded, the sharing server, the AI assistant server, sync and every
internet request are off, and the settings start at their defaults. **Nothing is saved to your settings file** in safe
mode, so your real settings are back at the next normal start. Your library and books are untouched.

## Open With and `--doc`

`bookx file.epub` (or *Open With* in the file manager) adds the book to the library; if Bookx is already running the
running window does it. `bookx --doc file.epub` adds the book if needed and opens it in the reader.

## Reporting a bug

Open an issue at [github.com/adhadse/Bookx/issues](https://github.com/adhadse/Bookx/issues) and paste the
[diagnostics](#copy-diagnostics). Include the version from *About Bookx → Troubleshooting* and say where you got Bookx
(a GitHub Releases file, which one, or built from source). Say what you did and what you expected. For a crash, start Bookx from a terminal with
`RUST_LOG=bookx=debug bookx` (Flatpak: `flatpak run --env=RUST_LOG=bookx=debug com.adhadse.Bookx`) and attach the output.

## bookx:// links

A link opens a book in the running Bookx (it starts Bookx when none is running):

| Link | Opens |
|---|---|
| `bookx://open?id=42` | the book with library id 42 (ids belong to one library) |
| `bookx://book/isbn:9780441172719` | the book that carries this identifier (`isbn:`, `doi:`, `uuid:` … or just the value) |

Put them in a note, a launcher or a script: `xdg-open 'bookx://book/isbn:9780441172719'`. If no book matches, Bookx
says so in a message. Characters in an identifier can be written as `%XX` (`doi%3A10.1000%2F182`).

!!! note "Limitations"
    - There is no automatic update check; download the next release from [GitHub Releases](https://github.com/adhadse/Bookx/releases/latest) (see [Install](install.md#update)) or rebuild from source.
    - Accessibility for screen readers is incomplete in the reader view (the text is drawn by Bookx itself).

## Questions and answers

??? question "How do I report a bug with the right information?"
    Open the main menu → **About Bookx** → **Troubleshooting**, copy the diagnostics (or use **Copy Diagnostics** in the
    command palette, ++ctrl+shift+p++) and paste it into a new issue on GitHub; it starts with the Bookx version. It contains no book titles, passwords or
    account names. For a crash, start Bookx from a terminal with `RUST_LOG=bookx=debug bookx` and attach the output.

??? question "Why is my library read-only?"
    Another running Bookx or `bookx-cli` has it open. Close the other one and reopen the library. A lock left by a crash on the
    same computer is taken over automatically; a lock from another computer is not, so check nothing else uses the library
    before removing `.bookx-lock`.

??? question "How do I get a deleted book or an old state back?"
    Open the main menu → **Library Health** and choose **Trash**. Deleted books wait there until you empty the trash, and the
    **History** list can undo recent changes. For older states restore a backup on the **Backups** page, see
    [Backup and restore](backup-and-restore.md).

??? question "How do I find and fix missing files or damage?"
    Main menu → **Library Health** → **Health**, then **Check Now**. Each problem offers what you can do about it. Removing a
    missing file only removes it from the catalogue (into the trash).

??? question "A book did not import. Why?"
    LIT and DRM-protected books cannot be read. Duplicates are skipped silently; main menu → **Library Health** →
    **Duplicates** shows the copies. For a damaged EPUB run `bookx-cli check book.epub` or open it in the editor and press
    ++f7++. A scanned PDF has no text to search; there is no OCR.

??? question "Why does Read Aloud do nothing?"
    Open the speaker popover: the line under the buttons says which voice is reading and why. The usual causes are a voice
    that is not downloaded yet (Preferences → **Text to Speech**), a missing `espeak-ng` package, no audio player, or an
    unreachable voice server. See [Read aloud](read-aloud.md).

??? question "Why does my e-reader not show up?"
    Make sure it is mounted (GNOME Files shows it); unlock a phone and choose file transfer. The **Devices** button appears in
    the library header only while something is connected; main menu → **Devices** is always there. Run
    `bookx-cli device-check <mount>`. See [Devices and sharing](devices-and-sharing.md).

??? question "Why is there no dictionary or no spelling check?"
    The reader uses StarDict dictionaries from `~/.local/share/bookx/dictionaries/`. The editor's spelling check uses the
    system Hunspell dictionaries: install the package for your language.

??? question "Wikipedia, translation or covers do not respond. Is Bookx offline?"
    These look-ups need the network. Check your connection, and check that **Offline mode** is off: main menu →
    **Preferences** → **Privacy**.

??? question "Does Bookx send anything to the internet?"
    Only when you ask it to: fetching metadata, Wikipedia and translation, *Add from the Web*, news, e-mail, and plugins that
    declare network permission. The Privacy page of Preferences lists every connection and has the Offline mode switch,
    see [Privacy](privacy.md).

??? question "Where are my settings and data?"
    See [Install](install.md#where-things-are-stored). Reading positions, highlights and bookmarks live in the library
    database, so they travel with the library folder.

??? question "The sync server was on, now it is off. What happened?"
    Bookx remembers the state of the server and starts it again at launch. If it did not start, Preferences → **Sync** shows
    why, for example a port already in use. See [KOReader highlights and notes](koreader-sync.md).

??? question "I get a warning about a network share or a sync folder."
    A library on a network file system or in a Dropbox-style folder can be damaged if two computers change it at once. Keep
    the library on a local disk and use backups or the OPDS server to reach it from elsewhere.

??? question "How do I open a book from a link or a script?"
    Use a `bookx://` link, for example `xdg-open 'bookx://book/isbn:9780441172719'`; see [bookx:// links](#bookx-links).
