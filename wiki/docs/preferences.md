# Preferences

Bookx keeps its settings in one window with a list of 12 pages on the left and the settings of the chosen page on the
right. Open it from the main menu (the hamburger button at the right of the library's header bar) → **Preferences**,
with ++ctrl+comma++, or from the **Book** menu of the book editor. ++ctrl+page-down++ and ++ctrl+page-up++ go to the
next and previous page, ++ctrl+f++ jumps to the search (which also finds the choices of a drop-down, such as *Dark*).

![Preferences](assets/preferences-light.png#only-light)
![Preferences](assets/preferences-dark.png#only-dark)

Changes apply as you make them; there is no Save button. Backups are not here: they live in **Library Health**, also in
the main menu, and the Library page has a row that opens it (see [Backup and restore](backup-and-restore.md)).

## What is on each page

| Page | What it holds |
|---|---|
| General | The colour scheme, **E-ink mode** (black on white, no animations, bars that stay, wide tap zones) the [interface language](#language) and **Settings Backup**: *Export Settings* saves your preferences to a JSON file (never passwords, tokens, the library folder or window size), *Import Settings* shows which settings the file would change before you confirm; restart Bookx afterwards to apply all of them |
| Library | The library folder (**Change** asks before it opens another folder, and names it), the **Continue reading banner** (Adaptive, Large, Compact or Hidden; see [the library](library.md#views)), *Library files* (where books are kept inside the folder, the path template, and a row that opens Library Health for backups, checks, trash and history) and [metadata lookup](library.md#fetch-metadata-for-new-books) (fetch for new books, the source cache) |
| Reading | Reader defaults: theme, layout, font size, justification, hyphenation, the book's own styling, a **Touch and e-ink readers** group (tap zones, large touch controls), footnote popups, the statistics in the footer (items, order, preset, position), the [reading ruler](accessibility.md#reading-ruler) and the comics settings |
| Dictionaries | The [dictionaries](reading.md#dictionaries) Bookx looks words up in (automatic download, the ones installed, getting another, your own folder), *Your vocabulary* and whether vocabulary terms are marked in the text |
| Editor | How the [book editor](editing.md#editor-preferences) looks and behaves: code font, tab width, margins, colour scheme (with your own schemes), automatic closing tags, completion, spell-check underlines and the live preview |
| Text to Speech | The voice engine, the voice, model downloads, the audio cache and the voice server: see [Read aloud](read-aloud.md#settings) |
| Sync | The settings for everything that syncs: a setup checklist for [Bookx sync](progress-sync.md) (server, account, plugin, test), what it does, USB devices, the library server (OPDS), and a collapsed **Advanced** row with every technical setting (server address, port and firewall, HTTPS, book matching, privacy and offline mode, reverse proxy, requirements). The **Always sync when connected** switch for USB e-readers is in [Devices → Settings](devices-and-sharing.md#settings-page), not here |
| E-mail | Mail server and addresses for *Send by E-mail* |
| Library → Metadata sources | which source wins for each field when several know it (title, authors, publisher, date, description, tags, language, series, cover; *Automatic* keeps the built-in order), and the **author name order**: *Automatic* leaves Chinese, Japanese and Korean names as written, *Last name, first name* always rearranges, *As written* never does |
| Extensions | Plugins, which can add metadata sources, file formats, conversion steps and device support (an empty list says where to put them and has a button that opens the folder), and **Patches**: small rule files that fix books when they are imported or opened; each has a switch and a **⋯** menu (dry run, apply earlier, apply later) |
| Privacy | [Offline mode](privacy.md#offline-mode) and the list of everything Bookx can connect to (plain text rows that the keyboard skips) |
| Keyboard Shortcuts | Every action of the library, the reader and the book editor with its keys: [change, reset, export and import them](shortcuts.md#changing-shortcuts) |
| MCP | The server that lets [AI assistants](ai-assistants.md) (the Model Context Protocol) search and read the library: switch, port (locked while the server runs), access token (making a new one asks first), whether they may change the library, ready-made settings for Claude, Cursor and opencode |

Settings that are not here: backups and library checks (main menu → **Library Health**), the *Always sync when
connected* switch (the Devices window → **Settings**), and the appearance of the
reader while you read (the reader's **Appearance** menu; the Reading page sets the defaults for new books).

## Section by section

Each page is one section of the window; these are the ones that have no page of their own.

### General

<figure markdown="1">
![Preferences, General](assets/preferences-general-light.png#only-light)
![Preferences, General](assets/preferences-general-dark.png#only-dark)
<figcaption markdown="1">Preferences → General: colour scheme and language.</figcaption>
</figure>

The General page is the place for the **Color scheme** (*Follow system*, *Light* or *Dark*), the **E-ink mode** switch
and the **Interface language** (see [Language](#language)). Switching E-ink mode on also adjusts the reader defaults
for e-ink; see [E-ink mode and touch](reading.md#e-ink-mode-and-touch).

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

The Library page starts with the **Library folder** and its **Change** button, then the banner, then *Library files*:
**Where books are kept** (the folder layout inside the library, with a **Path template** such as
`{author}/{series}/{title}` when you pick the template layout; changing it moves the files after a preview) and a row,
*Backups, check, trash and history*, that opens [Library Health](library.md#library-health). *Metadata lookup* closes the
page.

### Reading

<figure markdown="1">
![Preferences, Reading](assets/preferences-reading-light.png#only-light)
![Preferences, Reading](assets/preferences-reading-dark.png#only-dark)
<figcaption markdown="1">Preferences → Reading: the defaults of the reader (theme, layout, font size, hyphenation, tap zones).</figcaption>
</figure>

Preferences → Reading has the **Reader defaults** (*Theme*, *Layout*, *Font size*, *Justify text*, *Hyphenate words*,
*Use the book's own styling*), the **Touch and e-ink readers** group (*Tap zones*, *Large touch controls*), the footnote
popup switch, **Footer Statistics**, the **Reading ruler** and the comics settings. The same choices can be changed while
reading from the reader's **Appearance** menu; this page sets what a book starts with.

### Sync

<figure markdown="1">
![Preferences, Sync](assets/preferences-sync-light.png#only-light)
![Preferences, Sync](assets/preferences-sync-dark.png#only-dark)
<figcaption markdown="1">Preferences → Sync: the setup checklist first; everything technical is under Advanced.</figcaption>
</figure>

The Sync page walks you through four steps in its **Setup checklist** (turn on the Bookx sync server, create your sync
account, install the plugin on KOReader, test on the device). The server switch in step 1 is remembered, so the server
starts again by itself when you open Bookx; once you have registered or logged in, step 2 reads *Logged in as* your user
name. Below the checklist come *What Bookx sync does*, USB device sync and *OPDS server*. The
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

The Editor page has four groups: **Code** (font, tab width, line numbers, wrapping, the overview map, the *Colour
scheme*), **Editing** (closing tags, completions, *Underline misspelled words*), and **Preview** (its font and size,
*Follow the cursor*, and *Scrolling the preview scrolls the code*, which is on by default). The same page opens from the
editor's **Book** menu → **Preferences**.

### Keyboard Shortcuts

<figure markdown="1">
![Preferences, Keyboard Shortcuts](assets/preferences-shortcuts-light.png#only-light)
![Preferences, Keyboard Shortcuts](assets/preferences-shortcuts-dark.png#only-dark)
<figcaption markdown="1">Preferences → Keyboard Shortcuts: click a row to change its shortcut.</figcaption>
</figure>

The other sections are shown where their feature is described: [E-mail](devices-and-sharing.md#e-mail-and-kindle), [Extensions](plugins-and-patches.md#using-plugins), [Patches](plugins-and-patches.md#using-patches), [Text to Speech](read-aloud.md#settings), [Privacy](privacy.md#offline-mode) and [AI Assistants](ai-assistants.md#setting-up-from-preferences).

## Language

The **Interface language** row on the General page sets the language of the interface (a toast says *Language changed*): *System language* (the default, taken from `LANGUAGE`,
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
  (the ones the search leaves in the list) from anywhere in Preferences. They are listed, but cannot be changed, on the *Keyboard Shortcuts* page.
- Messages such as *Removed German dictionary* appear at the bottom of the window; the ones that can be taken back have an
  **Undo** button for a few seconds.
- In a narrow window the list of sections becomes an overlay. The button at the top left of the settings (or ++ctrl+f++)
  shows it; choosing a section hides it again.
- Every control has a name for screen readers; see [Accessibility](accessibility.md).

## Recommended settings

Opinionated starting points. Every one is a real setting; change what does not suit you. The page is the one in the
Preferences list unless another place is named.

**A typical reader**

- General → **Color scheme**: *Follow system*.
- Reading → **Hyphenate words**: on, for narrow columns and justified text (and **Justify text** on if you like even edges).
- Reading → **Layout**: *Two columns* on a wide window, *Single column* on a narrow one.
- Privacy → **Offline mode**: off, so covers and dictionaries arrive on their own.
- Dictionaries → **Download dictionaries automatically**: on.
- Main menu → Library Health → Backups: **Automatic backups** on (see [Backup and restore](backup-and-restore.md)).

**A student or researcher**

- Dictionaries → **Mark vocabulary terms in the text**: on, so your own definitions are underlined as you read.
- Dictionaries → **Download dictionaries automatically**: on, for books in other languages.
- Reading → **Footer Statistics**: choose the items you want in the footer, so you always see how far you are.
- Library → **Fetch metadata for new books**: on, so titles, authors and identifiers are filled in for citations (see
  [Export citations](library.md#export-citations)).
- Reading → **Show footnotes in a popup**: on, so you do not lose your place.
- Main menu → Library Health → Backups: a second place besides this computer, so highlights and notes survive.

**An e-ink, Kobo or KOReader user**

- General → **E-ink mode**: on. Then Reading → **Tap zones** and **Large touch controls** to taste.
- Sync → follow the **Setup checklist** to sync reading position, highlights and notes with KOReader over Wi-Fi; for a stock Kobo or
  Kindle use USB and switch **Always sync when connected** on in Devices → Settings.
- Editor → **Colour scheme**: a black-on-white scheme with *Follow light and dark mode* off, if you edit on an e-ink screen.

**A privacy-minded user**

- Privacy → **Offline mode**: on. Read the list *What Bookx can connect to* on the same page.
- Dictionaries → **Download dictionaries automatically**: off; install the dictionaries you want yourself (offline mode
  refuses downloads anyway).
- Library → **Fetch metadata for new books**: off.
- Sync → leave the server off unless you use it; MCP → **Let AI assistants connect**: off, and **Allow changes to the library**: off.
- Backups only to places you control, and encrypted (see [Backup and restore](backup-and-restore.md)).

**Someone with a big library**

- Back up to **two places**, one of them encrypted and not in the same building or on the same drive; check the result in
  Library Health → Backups (see [Backup and restore](backup-and-restore.md)).
- Run the **Check** in Library Health now and then, and after any big import.
- Library → **Where books are kept**: pick the layout you want while the library is small; changing it later moves every file.
- Library → **Continue reading banner**: *Compact* or *Hidden* to leave room for the books.

## Questions and answers

??? question "How do I start the Bookx sync server (and create the account, install the KOReader plugin, test it)?"
    Open **Preferences → Sync**. The **Setup checklist** has four steps. Step 1, **Turn on the Bookx sync server**: switch
    it on; the address for your e-reader (ending in `/bookx`) appears below it. Step 2, **Create your sync account**:
    type any user name and password and press **Register**; step 2 then shows *Logged in as* your name. Step 3,
    **Install the Bookx plugin on KOReader**: with the e-reader connected by USB, copy the plugin to it (or choose a
    folder), then restart KOReader. Step 4, **Test on the device**: in KOReader open a book → top menu → More tools →
    Bookx Sync → Sync now, and enter the address, user name and password. The details are in [Bookx sync](progress-sync.md).

??? question "How do I stop the Bookx sync server?"
    On **Preferences → Sync**, switch off the server in step 1 of the checklist. It stays off, also the next time you
    open Bookx, until you switch it on again. An e-reader that tries to sync then shows a connection error.

??? question "How do I make a backup right now?"
    Open the main menu, choose **Library Health** and pick **Backups**. Backups are not in Preferences; the Library
    page has a row, *Backups, check, trash and history*, that opens the same window. Backups include your book files:
    the first run copies everything, later ones add only what changed. See [Backup and restore](backup-and-restore.md).

??? question "How do I set up automatic backups?"
    In Library Health → Backups, switch on **Automatic backups** and choose how many snapshots to keep. The
    [Backup and restore](backup-and-restore.md) page explains the places a copy can go.

??? question "How do I back up to a USB drive or to Nextcloud?"
    Both are places you add in Library Health → Backups: a folder on the drive for the USB drive, and a folder of the
    synced Nextcloud client or a WebDAV address for Nextcloud. Step by step, with encryption, on
    [Backup and restore](backup-and-restore.md).

??? question "How do I restore a backup?"
    Library Health → Backups lists what you can restore; [Backup and restore](backup-and-restore.md#restoring)
    describes it, including restoring on a new computer.

??? question "How do I change the interface language?"
    Preferences → General → **Interface language**. Restart Bookx to change everything; open windows keep their
    language. Missing translations stay in English, and [Translating Bookx](translating.md) tells how to help.

??? question "How do I change the reading defaults (font, theme, hyphenation)?"
    Preferences → Reading, under **Reader defaults**: *Theme*, *Layout*, *Font size*, *Justify text* and
    *Hyphenate words*. A book that you already adjusted keeps its own settings; while reading you can change the same
    things from the reader's **Appearance** menu.

??? question "How do I turn on e-ink mode?"
    Preferences → General → **E-ink mode**. It gives black on white, no animations, bars that stay and wide tap zones.
    Adjust **Tap zones** and **Large touch controls** under Reading → *Touch and e-ink readers*. More in
    [E-ink mode and touch](reading.md#e-ink-mode-and-touch).

??? question "How do I install a dictionary?"
    Preferences → Dictionaries. Under **Get a dictionary** pick a language and press **Download**, or put your own
    StarDict dictionary into the folder that **Your own dictionaries** → **Open Folder** shows. Installed ones are listed
    with a remove button and an **Undo** toast. See [Dictionaries](reading.md#dictionaries).

??? question "How do I make Bookx download dictionaries by itself?"
    Preferences → Dictionaries → **Download dictionaries automatically**. When a book in a new language is added or
    opened, its dictionary is fetched in the background. Offline mode stops it.

??? question "How do I change a keyboard shortcut, or reset them?"
    Preferences → Keyboard Shortcuts: click a row, press the new keys. The arrow button on a row resets that action;
    under **Share and Reset**, **Reset All Shortcuts** brings every default back (with **Undo**). You can also
    **Export Shortcuts** and **Import Shortcuts**. See [Changing shortcuts](shortcuts.md#changing-shortcuts).

??? question "How do I install a plugin?"
    Preferences → Extensions → **Open plugins folder**, put the plugin's folder in it, go back and switch the plugin on.
    See [Using plugins](plugins-and-patches.md#using-plugins).

??? question "How do I install a patch?"
    On the same Extensions page, in the **Patches** group, press **Open patches folder**, drop the `.toml` file in, and
    switch it on. Try it first with **⋯** → **Dry Run on Your First Book**. See [Using patches](plugins-and-patches.md#using-patches).

??? question "How do I connect an AI assistant, and let it edit my library?"
    Preferences → MCP: switch on **Let AI assistants connect** (or skip it and use the command), then under **Connect an
    assistant** press **Command** or **Server** for your assistant and paste what is copied into its settings. To let it
    change things, switch on **Allow changes to the library**; Bookx asks first. See [MCP](ai-assistants.md).

??? question "How do I change where the library lives?"
    Preferences → Library → **Library folder** → **Change**. Bookx names the folder and asks before it opens it. To keep
    several libraries side by side, see [Several libraries](library.md#several-libraries). Where the files are placed
    *inside* the library is the **Where books are kept** row on the same page.

??? question "How do I set up e-mail for sending books?"
    Preferences → E-mail: fill in **Server**, **Port**, **Encryption**, **User name**, **Password**, **Your address** and
    **Send books to** (for a Kindle, your Send to Kindle address), then press **Send a Test Message**. Use an app password
    if your provider offers one. See [E-mail and Kindle](devices-and-sharing.md#e-mail-and-kindle).

??? question "How do I turn off every network feature?"
    Preferences → Privacy → **Offline mode**. Online look-ups and downloads stop; servers you set up yourself (mail,
    WebDAV, sync) keep working, so leave those switched off or empty too. The list on the same page shows everything
    Bookx can connect to. See [Privacy](privacy.md).

??? question "How do I change the editor's font or colour scheme?"
    Preferences → Editor → **Code**: the **Font** row picks the code font, **Colour scheme** the theme (or switch on
    *Follow light and dark mode*). The preview has its own **Text font** under **Preview**.

??? question "How do I turn spell checking in the editor on or off?"
    Preferences → Editor → **Editing** → **Underline misspelled words**. To check a whole book, use the editor's
    **Tools** menu; see [editor preferences](editing.md#editor-preferences).

??? question "How do I stop the preview from scrolling my code?"
    Preferences → Editor → **Preview** → **Scrolling the preview scrolls the code**. It is on by default; switch it off
    and only scrolling the code moves the preview.

??? question "How do I choose a read aloud voice?"
    Preferences → Text to Speech: pick the **Voice engine** and the **Voice**, press **Play** on *Test the voice* to
    hear it, and get more voices under **Voices and models**. To use a voice server, fill in **Voice server**. See
    [Read aloud settings](read-aloud.md#settings).

??? question "How do I delete the voice cache?"
    Preferences → Text to Speech → **Audio cache size** → **Clear**. Downloaded voices stay; the stored sentences are
    made again when needed.

??? question "How do I change how the footer shows my reading statistics?"
    Preferences → Reading → **Footer Statistics**: the preset, items, order, position and size. The same dialog can be set
    for one book only. See [The footer](reading.md#the-footer).

??? question "How do I change the Continue reading banner?"
    Preferences → Library → **Continue reading banner**: *Adaptive*, *Large*, *Compact* or *Hidden*.

??? question "How do I make the library fetch metadata for new books?"
    Preferences → Library → **Fetch metadata for new books**. After an import, only empty fields are filled in. It
    needs the network, so Offline mode stops it. **Source cache** → **Clear** forgets earlier answers.

??? question "How do I search for a setting?"
    Press ++ctrl+f++ in Preferences and type; every page is searched, including drop-down choices. ++ctrl+k++ opens the
    command palette, where a setting can be opened directly. See [Searching](#searching).

??? question "How do I set the sync on a USB e-reader to happen by itself?"
    That switch is not in Preferences: open the Devices window → **Settings** and switch on **Always sync when
    connected**. See [Devices and sharing](devices-and-sharing.md#settings-page).

??? question "How do I change my sync account or password?"
    Preferences → Sync → step 2, **Create your sync account**; use **Log in** for an existing account or **Register**
    for a new one. The server switch in step 1 must be on first.

??? question "How do I make the settings window easier to use with the keyboard?"
    ++tab++ moves between the search, the list of pages and the settings; ++ctrl+page-down++ and ++ctrl+page-up++ change
    page. See [Keyboard and small windows](#keyboard-and-small-windows).

??? question "How do I reset all settings?"
    There is no reset button for everything. Close Bookx and rename the settings file, `~/.config/Bookx/config.json`
    (see [Install](install.md)); Bookx starts with its defaults the next time. Your books, catalogue, backups and
    keyboard shortcuts (`shortcuts.json`) are in other places and stay. Only parts have buttons: *Reset All Shortcuts*
    for keys, **Clear** for caches.

??? question "How do I turn off the footnote popups?"
    Preferences → Reading → **Show footnotes in a popup**.

??? question "How do I show a reading ruler?"
    Preferences → Reading → **Reading ruler** → **Show the reading ruler**, with **Lines in the band** and
    **Dimming** under it. See [Reading ruler](accessibility.md#reading-ruler).
