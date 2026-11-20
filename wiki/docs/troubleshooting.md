# Troubleshooting and FAQ

## The library opens read-only

Another running Bookx (or `bookx-cli`) has the library open. The message names who has it. Close the other one and
reopen the library. A lock left behind by a crashed Bookx on the same computer is taken over automatically; a lock
from another computer never is, so make sure nothing else is using the library before you remove `.bookx-lock`.

## I get a warning about a network share or a sync folder

A library on a network file system, or in a Dropbox/Syncthing-style folder, can be damaged if two computers change it
at once. Keep the library on a local disk and use [backups](library.md#library-health) or the
[share feature](devices-and-sharing.md#library-server) to reach it from elsewhere.

## A book did not import

- LIT is not supported, and Bookx tells you so. A PDF imports: its pages open in the PDF view and its text is reflowed into an EPUB. A scanned PDF has no text to search or reflow (there is no OCR yet); a password-protected PDF imports and asks for the password when you open it.
- DRM-protected books cannot be read.
- Duplicates are skipped silently. Open **Library Health → Duplicates** to see copies.
- Run `bookx-cli check book.epub` to see what is wrong with an EPUB, or open it in the [editor](editing.md) and press
  ++f7++.

## Something looks wrong after a metadata change or layout change

*Library Health* → *History* has **Undo**. A deleted book is in the **Trash**. A catalogue backup can be restored
from *Library Health* → *Backups*. See [Library](library.md#library-health).

## The library check reports missing files

Run the check from *Library Health* and read what each problem offers. Removing a missing file only removes it from
the catalogue (into the trash); it does not touch other files.

## Read aloud does nothing

Open the speaker popover: the line under the buttons says which voice is reading and why it is not the one you chose.

* *Kokoro is not downloaded* / *no Piper voice is downloaded*: download it in **Preferences → Text to Speech**.
* *espeak-ng is not installed*: Kokoro and Piper need the `espeak-ng` package.
* *no audio player found*: install PulseAudio or PipeWire tools (`pacat`, `pw-cat`) or `alsa-utils` (`aplay`).
* *cannot reach the voice server*: start the server, check the address, use **Test** on the settings page.
* *Preparing voice* stays at 0% or says you are offline: the default voice (about 130 MB) is downloaded the first time you
  press Read Aloud. Connect to the internet and press *Try again* (the part already downloaded is kept), or choose
  an external voice server in **Preferences → Text to Speech**.
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
- Run `bookx-cli device-check <mount>` and look at what it reports.
- Device support is tested only with simulated devices. Please report what you see, including the output of
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

## Reporting a bug

Open an issue at [github.com/adhadse/Bookx/issues](https://github.com/adhadse/Bookx/issues) and paste the
[diagnostics](#copy-diagnostics). Say what you did and what you expected. For a crash, start Bookx from a terminal with
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
    - There is no automatic update check; rebuild from source to update.
    - Accessibility for screen readers is incomplete in the reader view (the text is drawn by Bookx itself).
