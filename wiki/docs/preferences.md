# Preferences

Open **Preferences** from the main menu or with ++ctrl+comma++. The window has a list of sections on the left and the
settings of the chosen section on the right. ++ctrl+page-down++ and ++ctrl+page-up++ go to the next and previous
section, ++ctrl+f++ jumps to the search (which also finds the choices of a drop-down, such as *Dark*).

![Preferences](assets/preferences-light.png#only-light)
![Preferences](assets/preferences-dark.png#only-dark)

| Section | What it holds |
|---|---|
| General | Colour scheme, **E-ink mode** (black on white, no animations, bars that stay, wide tap zones) and the [interface language](#language) |
| Library | Where the books are kept (**Change…** asks before it opens another folder, and names it), the **Continue reading banner** (Adaptive, Large, Compact or Hidden; see [the library](library.md#views)), library health, [backups of the catalogue](backup-and-restore.md) and [metadata lookup](library.md#fetch-metadata-for-new-books) (fetch for new books, the source cache) |
| Reading | Reader defaults: theme, layout, font size, justification, hyphenation, a **Touch and e-ink readers** group (tap zones, large touch controls), footnote popups, the statistics in the footer (items, order, preset, position), the [reading ruler](accessibility.md#reading-ruler) |
| Dictionaries | The [dictionaries](reading.md#dictionaries) Bookx looks words up in, and *Your vocabulary* |
| E-mail | Mail server and addresses for *Send by E-mail* |
| Sync | The settings for everything that syncs: a setup checklist for [Bookx sync](progress-sync.md) (server, account, plugin, test), what it does, the library server (OPDS), and a collapsed **Advanced** row with every technical setting (server address, port and firewall, HTTPS, book matching, privacy and offline mode, reverse proxy, requirements). Persistent USB-device auto-sync is a switch in [Devices → Settings](devices-and-sharing.md#settings-page); the connection banner's **Sync** is one-shot. It was “Devices and Sharing” and “Sync” before; searching either name finds it |
| Extensions | Plugins, which can add metadata sources, file formats, conversion steps and device support (an empty list says where to put them and has a button that opens the folder), and **Patches**: small rule files that fix books when they are imported or opened; each has a switch and a **⋯** menu (dry run, apply earlier, apply later) |
| AI Assistants | The server that lets [AI assistants](ai-assistants.md) search and read the library: switch, port (locked while the server runs), access token (making a new one asks first), whether they may change the library, ready-made settings for Claude, Cursor and opencode |
| Privacy | [Offline mode](privacy.md#offline-mode) and the list of everything Bookx can connect to (plain text rows that the keyboard skips) |
| Editor | How the [book editor](editing.md#editor-preferences) looks and behaves: code font, tab width, margins, colour scheme (with your own schemes), automatic closing tags, completion, preview font |
| Keyboard Shortcuts | Every action of the library, the reader and the book editor with its keys: [change, reset, export and import them](shortcuts.md#changing-shortcuts) |

Other features add their own sections or rows here when they are available, for example backup. **Text to Speech** holds
the voice engine, speed, model downloads and the voice server: see [Read aloud](read-aloud.md).

## Section by section

Each section is one page of the window; these are the ones that have no page of their own.

### General

<figure markdown="1">
![Preferences, General](assets/preferences-general-light.png#only-light)
![Preferences, General](assets/preferences-general-dark.png#only-dark)
<figcaption markdown="1">Preferences → General: colour scheme and language.</figcaption>
</figure>

### Library

<figure markdown="1">
![Preferences, Library](assets/preferences-library-light.png#only-light)
![Preferences, Library](assets/preferences-library-dark.png#only-dark)
<figcaption markdown="1">Preferences → Library: the library folder, the banner, where books are kept, and metadata lookup.</figcaption>
</figure>

<figure markdown="1">
![The Continue reading banner setting](assets/prefs-banner-setting-light.png#only-light)
![The Continue reading banner setting](assets/prefs-banner-setting-dark.png#only-dark)
<figcaption markdown="1">The *Continue reading banner* row: automatic, large, compact or hidden.</figcaption>
</figure>

### Reading

<figure markdown="1">
![Preferences, Reading](assets/preferences-reading-light.png#only-light)
![Preferences, Reading](assets/preferences-reading-dark.png#only-dark)
<figcaption markdown="1">Preferences → Reading: the defaults of the reader (theme, layout, font size, hyphenation, tap zones).</figcaption>
</figure>

### Sync

<figure markdown="1">
![Preferences, Sync](assets/preferences-sync-light.png#only-light)
![Preferences, Sync](assets/preferences-sync-dark.png#only-dark)
<figcaption markdown="1">Preferences → Sync: the setup checklist first; everything technical is under Advanced.</figcaption>
</figure>

The page starts with a **Setup checklist**, a stepper of four numbered steps (turn on the Bookx sync server, create your sync
account, install the plugin on KOReader, test on the device), then *What Bookx sync does* and *Library servers (OPDS)*. The
**Always sync when connected** switch for a USB e-reader is in [Devices → Settings](devices-and-sharing.md#settings-page),
off by default; the library banner's **Sync** button only syncs once. The last row, **Advanced**, is collapsed; open it for
the server address, port and firewall, HTTPS and certificates, how books are matched, the status lines, the request limits,
the link to the reverse-proxy guide, the Requirements table (which device needs what, over which connection) and the link
to Privacy and offline mode. Main menu → **Sync** shows the task/status view and follows these switches at once. See
[Bookx sync](progress-sync.md) and [Which sync do I need?](devices-and-sharing.md#which-sync-do-i-need).

### Editor

<figure markdown="1">
![Preferences, Editor](assets/preferences-editor-light.png#only-light)
![Preferences, Editor](assets/preferences-editor-dark.png#only-dark)
<figcaption markdown="1">Preferences → Editor: how the code view looks and behaves.</figcaption>
</figure>

### Keyboard Shortcuts

<figure markdown="1">
![Preferences, Keyboard Shortcuts](assets/preferences-shortcuts-light.png#only-light)
![Preferences, Keyboard Shortcuts](assets/preferences-shortcuts-dark.png#only-dark)
<figcaption markdown="1">Preferences → Keyboard Shortcuts: click a row to change its shortcut.</figcaption>
</figure>

The other sections are shown where their feature is described: [E-mail](devices-and-sharing.md#e-mail-and-kindle), [Extensions](plugins-and-patches.md#using-plugins), [Patches](plugins-and-patches.md#using-patches), [Text to Speech](read-aloud.md#settings), [Privacy](privacy.md#offline-mode) and [AI Assistants](ai-assistants.md#setting-up-from-preferences).

## Language

*General → Language* sets the language of the interface (a toast says *Language changed*): *System language* (the default, taken from `LANGUAGE`,
`LC_ALL`, `LC_MESSAGES` or `LANG`), English, Español, Deutsch, Français or हिन्दी. Open windows keep their language; restart Bookx to change everything (menus and the main window are built at start-up). Text that
is not translated yet stays in English, and French and Hindi are starting files with the most visible text only.
Numbers, counts ("1 book", "3 books") and dates follow the language. Right-to-left languages mirror the window.

Want another language or a better translation? See [Translating Bookx](translating.md).

## Searching

Type in the search entry above the list (or press ++ctrl+f++ anywhere in Preferences). Every section is searched at
once: by the name of a setting, its description, the choices it offers (searching for *dark* finds *Color scheme*), the name of its group and the name of its section. The list then shows
only the sections that have a match, the first one opens and the matching settings are highlighted. Several words must
all match. ++escape++ clears the search; a second ++escape++ closes Preferences.

![Searching in Preferences](assets/preferences-search-light.png#only-light)
![Searching in Preferences](assets/preferences-search-dark.png#only-dark)

You can also use the command palette (++ctrl+k++): choosing a setting there opens Preferences on that setting.

## Keyboard and small windows

- ++tab++ moves between the search entry, the list of sections and the settings; ++up++ and ++down++ move through the
  sections, which open as you move. ++ctrl+page-down++ and ++ctrl+page-up++ open the next and the previous section
  (the ones the search leaves in the list) from anywhere in Preferences.
- Messages such as *Removed German dictionary* appear at the bottom of the window; the ones that can be taken back have an
  **Undo** button for a few seconds.
- In a narrow window the list of sections becomes an overlay. The button at the top left of the settings (or ++ctrl+f++)
  shows it; choosing a section hides it again.
- Every control has a name for screen readers; see [Accessibility](accessibility.md).
