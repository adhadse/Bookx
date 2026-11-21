# Devices and sharing

!!! warning "Hardware tested so far"
    Only one reader has been tried: a **Kobo Libra Colour, firmware 4.46.23852** (USB, read-only checks and a
    send into a scratch folder). Everything else on this page (other Kobo models, Kindle, PocketBook, Boox, phones,
    cloud folders) has been tested against simulated devices only. Keep a copy of anything important on the device
    before you send to it.

## The Devices button

<figure markdown="1">
![The connected e-reader banner in the library](assets/library-device-banner-light.png#only-light)
![The connected e-reader banner in the library](assets/library-device-banner-dark.png#only-dark)
<figcaption markdown="1">Plug in an e-reader and the library shows *E-reader connected* with a one-shot *Sync* button.</figcaption>
</figure>


While a Kobo, Kindle, other e-reader or a phone is connected, a **Devices** button with the device's name appears in
the library's header bar (it is hidden when nothing is connected). Its tooltip is **Connected e-reader: sync, send and
manage books**. It opens the Devices window; the main menu has a **Devices** entry too, which works with or without a
device. The book's right-click menu has one device entry, **Send to Device**: it
opens the Devices window with the books you selected already in its *Books to send* list. It is always there; without a
device it is greyed out, with the hint “No device connected”.

<figure markdown="1">
![A connected device: the Devices button and the banner](assets/library-device-light.png#only-light)
![A connected device: the Devices button and the banner](assets/library-device-dark.png#only-dark)
<figcaption markdown="1">The Devices button and the connection banner.</figcaption>
</figure>

When a device connects, a banner at the top of the library says **“E-reader connected”** and offers **Sync**. Pressing Sync shows what
will happen, then syncs this once. Highlights come with the colour and style they have on the device (see *Colours and
styles from your e-reader* in [Annotations](annotations.md)). The banner can be dismissed; there is no **Always** or
**Not now** answer. To sync every time a device connects, switch on **Always sync when connected** in
the Settings page of the Devices window ([below](#settings-page)) instead.

## Syncing progress, highlights and notes

Syncing is **always progressive and keeps all information**:

- Reading progress only ever moves forward. A book finished on one side is finished on both. If the device is behind
  the library, the device keeps its own progress and you are told.
- Highlights and notes are united: one that exists on one side only is added to the other, one that exists on both
  is not duplicated, and when only one side has a note on it the note is kept.
- **Nothing is ever deleted or lowered** because the other side does not have it.
- If both sides have the same highlight with **different notes**, nothing is overwritten: the summary lists the
  conflict (the highlight with both notes) and says that Bookx keeps its note and the device keeps its own.
- If the device is ahead but your library copy was read **more than a day more recently**, Bookx assumes you
  started the book again and leaves the progress alone, and says so.

Before anything happens a summary shows what goes where, with counts.

Highlights from the device are placed in the book by searching for their text; one that cannot be found stays on
the device and is reported in the summary. Highlights go to the device as a KOReader notes file next to the book,
and only when the device copy has none: **a notes file that is already there is never touched**. The summary says
how many highlights stay in Bookx for that reason. Reading progress is never written to the device.

**Always sync when connected** is off by default. You find it in the Devices window (the Devices button, or main menu →
**Devices**): switch to its **Settings** page and look under **Sync**. With it on, syncing starts as soon as a device
appears, without asking, and a message tells you what was done. The banner's Sync button never turns this on; it only
syncs once. Bookx Preferences → Sync has a *USB device sync* group with the notes switch and a button that opens the
Devices window; the other sync settings (the Bookx sync server, the account, the library server) live there too.

## Send to device

Select books in the library, right-click → **Send to Device** (or open the Devices window with the Devices button or from the main menu).
On the **Books** page, the **Books to send** group shows exactly what will happen to each book, before anything is
copied:

- the **format** it will have on the device and whether it is converted: *EPUB → KEPUB (converted)*,
  *EPUB (sent as is)*, *EPUB → AZW3 (converted)*;
- **+ 3 notes added as a last section** when the book has highlights, notes or bookmarks (see below);
- its size, and a **warning** when the book file is missing, is bigger than the free space, or is already on the
  device (sending would add a second copy).

With nothing selected, the group shows one row: **Select books in the library, then choose Send to Device**. There is
no *Send 0 Books* button and no empty selection popover. With books selected, each row has a check box at the far
left. Untick a book to leave it out; a book with a warning starts unticked. Click anywhere on a row or tile to tick it
and Shift-click ticks a range. The button says **Send 1 Book** or **Send N Books** and sends only the ticked ones, with
the very format decisions the list shows. If a conversion fails, the next format the device opens is tried.

**Your notes go along.** A book that has highlights, notes or bookmarks is sent with them: the copy **on the device**
gets an extra “My notes” section at the end (readable in any e-reader app), and for EPUB and KEPUB readers a KOReader
notes file is written next to it, so KOReader lists them too (as bookmarks and notes; KOReader keeps jump targets at
chapter level, so they jump to the start of the chapter; Bookx never writes a position inside the chapter that it cannot work out, so nothing can end up at the wrong words). Your library copy is not changed. Switch **Add notes to
the copy sent to the device** off to send books without them. It is under *Sending options* on the Settings page of the
Devices window, and also in Preferences → Sync under *USB device sync*.

This is **one-way, library → device**. The merge in both directions is the sync that runs when a device connects
(above); sending never reads anything back.

## The Devices window

Open it with the **Devices** button in the header bar, or main menu → **Devices**. The window has two pages, **Books**
and **Settings**.

### Books page

<figure markdown="1">
![The Books page of the Devices window](assets/device-window-light.png#only-light)
![The Books page of the Devices window](assets/device-window-dark.png#only-dark)
<figcaption markdown="1">The Books page: books on the device, sync and selection controls, and books to send.</figcaption>
</figure>


With no device connected, the Books page shows a status page: **No Device Connected** — **Connect your e-reader with
USB, or add a WebDAV folder below**. The Settings page is still available, so you can add a WebDAV folder before a
device appears.

When a device is connected, it is shown as a row titled **Device**, with its name and free space (main memory is told
apart from an SD card).

**Books on the device** shows the books with their covers. Covers are taken from the book files (EPUB and KEPUB;
other formats show a coloured placeholder) and cached, so the next visit is fast. The group has a **Sync Reading
State** suffix: it shows the summary above, then merges progress, highlights and notes (matching is by title). Its
other controls are:

- a **Select All / Select None** popover for the books on the device;
- one view button, **Show as a Grid** or **Show as a List**. It changes only this list, and the choice is remembered;
- check boxes that appear on hover and stay visible for selected books (tiles: check box top-left; list: check box far
  left, covers in a fixed slot so everything lines up). Click anywhere on a row or tile to tick it and Shift-click
  ticks a range.

At the bottom of the group, **Remove from Device** becomes active with a selection and asks **“Remove N Books from
the Device?”** before deleting. This deletion is not undoable.

**Books to send** is the second group (above). With no library books selected it is just the status row **Select
books in the library, then choose Send to Device**; with books selected its button says **Send 1 Book** or **Send N
Books**.

<figure markdown="1">
![A detail of the Books page](assets/device-window-detail-light.png#only-light)
![A detail of the Books page](assets/device-window-detail-dark.png#only-dark)
<figcaption markdown="1">Books on the device: covers, the grid/list button and the selection controls.</figcaption>
</figure>

### Settings page

<figure markdown="1">
![The Settings page of the Devices window](assets/device-window-settings-light.png#only-light)
![The Settings page of the Devices window](assets/device-window-settings-dark.png#only-dark)
<figcaption markdown="1">The Settings page: sending options, background sync and the WebDAV folder.</figcaption>
</figure>


- **Sending options**: the **Format** to send (if a conversion fails, the next format the device opens is tried), the
  **Save path** template such as `{author}/{series}/{title}`, and for Kobo an optional **Shelf** for sent books (the
  Kobo database is backed up before it is changed). **Add notes to the copy sent to the device** adds your
  highlights and notes to the copy on the device.
- **Sync**: **Always sync when connected**, off by default (this is the only place for it). The “?” explains that this syncs in the background every
  time a device appears; the banner's Sync button stays a one-shot action.
- **Cloud folder (WebDAV)**: enter the URL, user name and password to use a WebDAV folder as a device. Sending never
  overwrites an existing file and the password is kept in the system keyring. To keep *backups* in a WebDAV folder,
  see [Backup and restore](backup-and-restore.md).

Check a mounted device from the terminal with `bookx-cli device-check <mount>` (add `--all` to check every book on
it, not only the folder Bookx sends to). The check never writes to the device. See [Command line](command-line.md).

### Kobo: what is known to work

Verified on a Kobo Libra Colour (firmware 4.46.23852, library database schema version 176):

- The Kobo is recognised, with its model, firmware version and free space. Bookx reads the library database from a
  copy, so looking at the device does not change any file on it.
- Books converted to **KEPUB** and sent from Bookx arrive complete (checked byte for byte), with a valid structure
  (Kobo spans, unique ids, the text unchanged) and a file name the Kobo's FAT file system accepts. With notes the “My notes” section and the KOReader sidecar are written next to the book.
- Reading progress, finished books and last-read dates are read from KOReader's sidecar files (`<book>.sdr`), and
  from the Kobo database for books the Kobo itself imported. Highlights and notes in KOReader sidecars can be read.

Limits you should know:

- The Kobo only lists books it has imported after you eject it. Books copied into the root of the device and read in
  KOReader (the usual Calibre set-up) may never be in the Kobo's own database; then their progress comes from the
  KOReader files only.
- The Kobo also lists every file under `.adds` (KOReader's own files) in its database; Bookx ignores those.
- Putting sent books on a **Kobo shelf** changes the Kobo database. It was only tried on a copy of a real database, not
  on the device itself, and it is not known whether the Kobo keeps a shelf entry for a book it has not imported yet.
  The option is off unless you give a shelf name.
- Whether the Kobo then imports and displays a sent KEPUB (cover, reading-time line) has not been checked on the
  device yet.

### Phones (MTP)

An Android phone mounted through your desktop (GNOME Files, gvfs) appears as a device, under
`/run/user/<uid>/gvfs/mtp:…`. Limits: free space is often unknown, a locked phone shows nothing, and there is no
Kobo database handling.

### Cloud folders (WebDAV)

Open the Devices window (main menu → **Devices**), go to its **Settings** page and fill in **Cloud folder (WebDAV)**:
enter the address, user name and password, press **Check Connection**, and the folder behaves like a device. Sending never overwrites an existing file. The password is kept in the
system keyring. To keep *backups* in a WebDAV folder, see [Backup and restore](backup-and-restore.md).

## Which sync do I need?

Everything that moves books, progress, highlights or notes between Bookx and another device is under one name,
**Sync**: **main menu → Sync** answers *what should I use, when* with a status card and three choices; **Bookx Preferences →
Sync** holds the setup (the main menu has **Preferences**).

| Use this when… | **Bookx sync with KOReader (Wi-Fi)** | **USB device** | **Share books (OPDS)** |
|---|---|---|---|
| Your e-reader… | runs KOReader (Kobo, Kindle, Android, PocketBook …) | is connected by a cable, with or without KOReader | is an app or reader that browses an OPDS catalogue |
| Connection | Wi-Fi or your home network | USB cable (or a phone mounted in Files) | Wi-Fi or your home network |
| What moves | reading position; with the Bookx plugin also highlights, notes and bookmarks | reading progress, highlights and notes, both ways (a merge); books you send | books and the catalogue only |
| Bookx needs | the server on and a sync account (Register) | nothing to switch on | the library server on |
| The e-reader needs | KOReader, Progress sync or the Bookx plugin set to Bookx’s `/bookx` address | to be mounted | an app that opens the address |

**Stock Kobo and Kindle Wi-Fi sync is not supported**: readers on their own firmware sync over the USB cable only.
Over Wi-Fi Bookx syncs with KOReader.

**Bookx sync with KOReader (Wi-Fi)** is described in [Bookx sync (extended KOSync)](progress-sync.md) (the setup
checklist, the account, how books are matched) and [KOReader highlights and notes](koreader-sync.md) (the plugin).
Bookx’s own reader uses the same account: it asks “Continue from *Kobo Libra* at 63%?” when you open a book that was read
further elsewhere.

**USB device** (cable): Bookx reads the progress and the highlights, notes and bookmarks the device keeps (a Kobo’s dog-ears,
KOReader’s page bookmarks), from the Kobo’s own database or from KOReader’s `.sdr` files if KOReader is installed on it, and
merges them with the library. A bookmark becomes a Bookx bookmark at the same place (a rough one is marked *approximate place*),
is added once, and is not brought back once you delete it in Bookx. Without any
readable state nothing happens. Progress only moves forward, notes are united, nothing is deleted, and reading progress
is never written to the device. *Notes sent with a book* (Devices window, *Send to Device*) add your highlights and notes
to the copy on the device as a “My notes” section and a KOReader notes file; your library copy is never changed.

**Share books (OPDS)** is the library server: see [Library server](#library-server).

**Import Highlights** brings the highlights from a Kindle’s `My Clippings.txt`, KOReader’s notes, a Kobo database or
calibre into the library; see [Highlights and notes](annotations.md).

## E-mail and Kindle

<figure markdown="1">
![Preferences, E-mail](assets/preferences-email-light.png#only-light)
![Preferences, E-mail](assets/preferences-email-dark.png#only-dark)
<figcaption markdown="1">E-mail: server, account and the address books are sent to. The password goes to the system keyring.</figcaption>
</figure>


Open the main menu → **Preferences**, choose the **E-mail** page and fill it in (mail server, port, encryption STARTTLS or TLS, user name, password, your address and the
address to send to, for example your Send-to-Kindle address). Then right-click a book → *Send by E-mail*. Bookx checks the
50 MB Kindle limit before sending.

The password is kept in the system keyring (see [Backup and restore → Passwords](backup-and-restore.md#passwords)). Use an app password.

## Library server { #library-server }

<figure markdown="1">
![The Sync window](assets/library-share-light.png#only-light)
![The Sync window](assets/library-share-dark.png#only-dark)
<figcaption markdown="1">Main menu → Sync: a status card with one main button, and three choices (Bookx sync with KOReader, USB device, Share books). Off by default.</figcaption>
</figure>

Main menu → **Sync** → **Share books (OPDS)** → **Start library server** serves the library to other devices on your
network; Bookx **Preferences → Sync** has a *Library servers (OPDS)* group with the same switch and the address with a copy button:

- **OPDS** catalogue and a **web page** to browse and download books. Any OPDS reader app can use it.
- A **password is required** before other devices can connect. The user name is `bookx`. In Preferences → Sync, the collapsed **Advanced** row holds
  *Allow other devices on the network*, the library password, the **port** (allow it in your firewall) and **HTTPS**.
- **Bookx sync** shares the server: the setup checklist turns on network access for you and makes the library password
  if there is none. See [Bookx sync (extended KOSync)](progress-sync.md).
- **HTTPS**: use your own certificate and key (PEM files) or let Bookx make a self-signed one. Clients must trust a
  self-signed certificate themselves. A button creates a new one.

The same server runs without the app: `bookx-cli serve --port 8080 --user me:secret --kosync kosync.json` (the `--kosync` file keeps the Bookx sync accounts).

To reach the server by a name, with proper HTTPS, or from outside the house, keep Bookx on your own computer and put a
reverse proxy in front of it: [Put Bookx behind a reverse proxy](reverse-proxy.md).

!!! note "Limitations"
    - Only the Kobo Libra Colour mentioned above has been tried on real hardware; the rest is untested.
      `docs/hardware-test-plan.md` in the repository lists what will be tried.
    - Writing a Kobo shelf has only been run against generated databases and a copy of a real one.
    - The covers of books on a device are read from the book files; the Kobo's own image cache is not used, and
      books that are not EPUB or KEPUB show a placeholder.
    - Dragging books onto the Devices button is not possible; select them in the library and use *Send to Device*.
    - Kindle's own reading state (binary `.sdr` data) is not read. KOReader on a Kindle works.
    - The Calibre wireless "smart device" protocol is not implemented.
    - MTP works through gvfs only; direct USB MTP is not done.
    - WebDAV was tested against an in-process server, not Nextcloud or another real service.
    - Without a keyring (GNOME Keyring, KWallet, KeePassXC) the passwords stay in the settings file, unencrypted; Bookx says so next to each one.

## Questions and answers

??? question "How do I send books to my Kobo or Kindle?"
    Plug the reader in with its USB cable. In the library select the books, right-click and choose **Send to Device**
    (or open main menu → **Devices**). On the **Books** page the *Books to send* list shows, for every book, the format it
    will have on the device, for example *EPUB → KEPUB (converted)* for a Kobo or *EPUB → AZW3 (converted)* for a
    Kindle, plus its size and any warning. Untick what you do not want and press **Send 1 Book** or **Send N Books**.
    Your library copy is never changed.

??? question "The Devices button is not there. What is wrong?"
    The **Devices** button in the header bar only appears while a device is connected. Make sure the reader is
    mounted (GNOME Files shows it); for a phone, unlock it and choose file transfer. The main menu → **Devices** entry is
    always there and opens the same window, with a *No Device Connected* page until something shows up. If it still does
    not appear, run `bookx-cli device-check <mount>` and look at what it reports.

??? question "How do I sync reading progress and highlights over USB?"
    When a device connects, a banner says **E-reader connected** with a **Sync** button. Press it: Bookx shows what
    will go where and then syncs once. You can also open the Devices window and press **Sync Reading State** in the *Books
    on the device* group. Progress only moves forward, highlights and notes from both sides are united and nothing is
    deleted. Reading progress is never written to the device.

??? question "How do I make it sync every time I plug the reader in?"
    Open main menu → **Devices**, switch to the **Settings** page and turn on **Always sync when connected** under
    **Sync**. It is off by default. Answering the banner with Sync never turns it on, that is a one-time sync.

??? question "Will my highlights and notes go onto the device when I send a book?"
    Yes, by default. The copy on the device gets an extra “My notes” section at the end, and for EPUB and KEPUB a KOReader
    notes file next to it. If you do not want that, switch off **Add notes to the copy sent to the device** on the Devices
    window's Settings page (*Sending options*) or in Bookx Preferences → Sync under *USB device sync*. A notes file
    that is already on the device is never overwritten.

??? question "How do I remove books from the device?"
    In the Devices window, on the Books page, tick the books in *Books on the device* (the grid/list button switches the
    view, Shift-click ticks a range) and press **Remove from Device**. Bookx asks “Remove N Books from the Device?” first.
    This cannot be undone.

??? question "How do I choose the format or folder layout on the device?"
    Devices window → **Settings** page → *Sending options*. **Format** is *Automatic (best for the device)* unless you pick
    one. **Save path** is a template such as `{author}/{series}/{title}`. On a Kobo you can give a **Shelf name** for sent
    books. If a conversion fails, Bookx tries the next format the device opens.

??? question "How do I add a WebDAV folder (Nextcloud and similar) as a device?"
    Open main menu → **Devices** and switch to **Settings**. Under **Cloud folder (WebDAV)** type the address, user name
    and password and press **Check Connection**. The folder then behaves like a device on the Books page. Sending never
    overwrites an existing file, and the password goes to the system keyring. Backups to WebDAV are set up separately,
    see [Backup and restore](backup-and-restore.md).

??? question "Which sync should I use: USB, KOReader or sharing?"
    Open main menu → **Sync**. It offers three choices: *Bookx sync with KOReader (Wi-Fi)* for readers that run KOReader,
    *USB device* for a cable, and *Share books (OPDS)* to let reader apps browse your library. Stock Kobo and Kindle
    firmware has no wireless sync with Bookx; use the cable for those. See [Which sync do I need?](#which-sync-do-i-need).

??? question "How do I send a book to my Kindle by e-mail?"
    Open main menu → **Preferences**, choose **E-mail** and fill in your mail server, account and your Send-to-Kindle
    address (use an app password). Then right-click a book → **Send by E-mail**. Bookx checks the 50 MB Kindle limit
    first.

??? question "How do I share my library with an OPDS reader app?"
    Open main menu → **Sync** and press **Start library server** in the *Share books (OPDS)* card, or use the switch in
    Bookx Preferences → Sync under *Library servers (OPDS)*. The address appears with a copy button. The user name is
    `bookx` and a password is required; port, HTTPS and network access are in the collapsed **Advanced** row. Keep it on
    your own network or put a [reverse proxy](reverse-proxy.md) in front.

??? question "Does it work with my phone?"
    An Android phone mounted through GNOME Files (MTP) appears as a device. A locked phone shows nothing, free space is
    often unknown and there is no Kobo database handling. Direct USB MTP without GNOME's mount is not supported.

??? question "Can I drag books onto the Devices button?"
    No. Select them in the library and use **Send to Device**.
