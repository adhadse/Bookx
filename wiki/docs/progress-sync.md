# Bookx sync (extended KOSync)

**Bookx sync (extended KOSync)** keeps your reading position, and with the Bookx plugin your highlights, notes and
bookmarks, in step between Bookx and KOReader over Wi-Fi. It is compatible with KOReader’s own KOSync progress
protocol, so KOReader’s built-in *Progress sync* works against it too; Bookx’s extension adds highlights, notes and
bookmarks over the same connection when the [Bookx plugin](koreader-sync.md) is installed in KOReader. From here on it is
just **Bookx sync**.

Everything is under **Sync** in the library's main menu (the hamburger button at the right of the library's header bar: what to use when, and one button) and **Preferences → Sync** (the setup).
Nothing goes to a cloud: the server is your own computer, a self-hosted server you enter, or the public one at
`sync.koreader.rocks` if you choose it.

## Set it up

<figure markdown="1">
![Preferences, Sync](assets/preferences-sync-light.png#only-light)
![Preferences, Sync](assets/preferences-sync-dark.png#only-dark)
<figcaption markdown="1">Preferences → Sync: the setup checklist first, everything technical under Advanced.</figcaption>
</figure>

*Preferences → Sync → **Setup checklist*** is a **stepper**: four numbered steps one under the other, each with a short status (for example *Off*, *alice*, *Copied*), a check mark when it is done and a line to the next step. The first step that is not done is open; click any step to open it (you can go back to a done one), click an open step to close it. The steps:

1. **Turn on the Bookx sync server.** Starts the Bookx sync server on this computer, opens it to your local network so
   the e-reader can reach it, and creates a library password if you have none (shown under **Advanced**). Turning the
   switch off stops the server again. While the server is open to the network, the Sync window (library main menu → Sync) shows
   the banner *Other devices on your network can reach this server*. The address for the e-reader appears below the switch, ending in `/bookx`
   (for example `http://192.168.1.5:8080/bookx`) with a copy button.
2. **Create your sync account.** Type any **User name** and **Password** and press **Register** (this also logs in).
   There is no existing KOReader account: you make it here. These are what you type into Bookx Sync on the e-reader.
   They are **not** the OPDS server password. The result shows next to the buttons; if the name is taken, choose
   another or press **Log in** if the account is yours.
3. **Install the Bookx plugin on KOReader.** **Copy to the device** puts it in KOReader’s plugins folder on the
   connected e-reader, or **Save to a folder** lets you copy it yourself. Without the plugin only the reading position
   is synced. See [KOReader highlights and notes](koreader-sync.md).
4. **Test on the device.** On the e-reader: open a book → top menu → More tools → **Bookx Sync** → **Sync now**.

To use another server (a self-hosted one or `https://sync.koreader.rocks`) open **Advanced** and type its **Server
address**; the same **User name**, **Password**, **Register** and **Log in** apply. Under **Advanced** you also find
**Sync the reading position** (the switch for Bookx’s own reader), **Match books by** and **This device is called**:

- **Match books by** must be the same as in KOReader (*Tools → Progress sync → Document matching method*):
  - **Partial MD5 of the file** (the default in both): books match when the files are byte-for-byte the same.
    Renaming is fine, converting or editing the book changes the id.
  - **File name**: books match when the file names are the same.
- **This device is called** is the name the other devices show (“Continue from *Bookx on my laptop*?”).

**Sync highlights, notes and bookmarks** (*What Bookx sync does*) is **on by default**. It is independent of the server: while the server is off nothing listens, and once the server is on (step 1) the Bookx plugin can sync highlights, notes and bookmarks without any further switch. Turn it off there if you only want the reading position.

The password is kept in the system keyring, like the OPDS server and e-mail passwords, or in Bookx's encrypted password
store when there is no keyring; never in the settings file (see [Passwords](backup-and-restore.md#passwords)).

## What happens

- **Opening a book** asks the server where it is up to. If another device read further than you are here by more
  than 1 %, Bookx asks **“Continue from Kobo Libra at 63%?”** with **Stay Here** and **Continue**. Nothing moves
  unless you choose Continue, and a position *behind* yours is never offered.
- **Reading** pushes your position a few seconds after you stop turning pages, and **closing the book** pushes at once.
- Until you have answered the question, a push that would take the server **backwards** (another device is further
  along) is held back instead of overwriting it.
- The reading position shown in your library follows what you read here; it is not changed by a position you did
  not accept.

Highlights, notes and bookmarks are not part of the stock KOSync protocol; Bookx’s extension syncs them when the
Bookx plugin is installed, see [KOReader highlights and notes](koreader-sync.md).

## Limits

- **The position is a percentage.** KOReader describes a place as a point in *its* layout of the book; Bookx cannot
  place that, so it turns the percentage into a chapter and a place in it with its own chapter weights. Expect to
  land within a page or two of where the other device was, further off in books with large images.
- **Bookx tells KOReader the start of the chapter** you are in (plus the exact percentage), so KOReader opens at the
  beginning of that chapter, not at the same line.
- Document ids are only equal for the same file (or file name). A converted or edited copy is a different book to
  the server.
- **Offline is fine.** If the server cannot be reached, the position is kept (one entry per book, the newest) and sent
  later: when Bookx starts, every minute, and as soon as the network comes back. The pause between tries grows from
  30 seconds to an hour. A server that refuses your login drops the entry instead of trying for ever. Opening a book
  still needs the network to ask where you were.
- Without the plugin only the position is synced. Use the plugin, or [Import Highlights](annotations.md), for highlights and notes.

## Using a reverse proxy

To serve the sync endpoint under a name and real HTTPS (`https://bookx.lan/bookx`), see
[Put Bookx behind a reverse proxy](reverse-proxy.md). Keep the `/bookx` prefix.

## Questions and answers

??? question "How do I start syncing with KOReader?"
    Open **Preferences → Sync** and work down the setup checklist: turn on the sync server, create an account, install the
    Bookx plugin on KOReader and test on the device. See [Set it up](#set-it-up).

??? question "Where do I see what is synced?"
    Open **Sync** in the library's main menu (the hamburger button in the library's header bar).

??? question "Which address do I type on the e-reader?"
    The address appears under the first switch of the checklist, ending in `/bookx`, with a copy button.

??? question "Why does Bookx ask me to continue from another device?"
    Another device read further than you are here by more than 1 %. Choose **Continue** to jump there or **Stay Here**; nothing
    moves unless you choose Continue.

??? question "Do I get my highlights and notes too?"
    Yes, with the Bookx plugin installed in KOReader and **Sync highlights, notes and bookmarks** on (the default). Without
    the plugin only the position is synced. See [KOReader highlights and notes](koreader-sync.md).

??? question "Why did the position land a few lines off?"
    The position travels as a percentage, so expect to land within a page or two of where the other device was.

??? question "What if I am offline?"
    Your position is kept and sent later. Opening a book still needs the network to ask where you were.

??? question "Which books match between devices?"
    Books match by the partial MD5 of the file (the default) or by file name; choose the same method in
    **Preferences → Sync → Advanced** and in KOReader.
